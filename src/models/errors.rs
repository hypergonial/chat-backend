use std::num::ParseIntError;

use aws_sdk_s3::error::{DisplayErrorContext, SdkError};
use axum::{
    Json,
    extract::multipart::MultipartError,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use derive_builder::UninitializedFieldError;
use jsonwebtoken::errors::ErrorKind;
use serde_json::json;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use thiserror::Error;

use crate::{external::fcm::FirebaseError, gateway::GatewayCloseCode};

/// An error response returned by the REST API.
#[derive(Debug, Clone)]
pub struct ErrResponse {
    status: StatusCode,
    error: String,
}

impl ErrResponse {
    pub fn new(status: StatusCode, error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            status,
        }
    }
}

impl ErrResponse {
    /// The HTTP status code of the error.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// The error message.
    pub fn error(&self) -> &str {
        &self.error
    }

    // TODO: Maybe think of something better than this?
    /// The hash of the error message. This is used to anonymize internal error messages in production.
    pub fn error_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.error.hash(&mut hasher);
        hasher.finish()
    }
}

// Depending on the build profile, we either return the full error message
// or a generic one in the case of an internal server error.
impl IntoResponse for ErrResponse {
    #[cfg(debug_assertions)]
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!(
                {
                    "error": self.error
                }
            )),
        )
            .into_response()
    }

    #[cfg(not(debug_assertions))]
    fn into_response(self) -> Response {
        let reason = if self.status == StatusCode::INTERNAL_SERVER_ERROR {
            format!("Internal Server Error - Ref: #{}", self.error_hash())
        } else {
            self.error
        };

        (
            self.status,
            Json(json!(
                {
                    "error": reason
                }
            )),
        )
            .into_response()
    }
}

/// Errors encountered during object initialization.
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum BuildError {
    /// A field was not initialized before calling `.build()` in a builder.
    #[error("Uninitialized field: {0}")]
    UninitializedField(&'static str),
    /// A validation check failed.
    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Illegal state: {0}")]
    IllegalState(String),
}

impl BuildError {
    const fn status_code(&self) -> StatusCode {
        match self {
            Self::ValidationError(_) => StatusCode::BAD_REQUEST,
            Self::IllegalState(_) | Self::UninitializedField(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<UninitializedFieldError> for BuildError {
    fn from(e: UninitializedFieldError) -> Self {
        Self::UninitializedField(e.field_name())
    }
}

impl From<String> for BuildError {
    fn from(e: String) -> Self {
        Self::ValidationError(e)
    }
}

impl IntoResponse for BuildError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self);
        }
        let body = Json(json!({
            "error": self.to_string()
        }));
        (status, body).into_response()
    }
}

/// Errors that can occur during authentication.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AuthError {
    /// Sent when the user provides invalid username/password.
    #[error("Invalid credentials")]
    InvalidCredentials,
    /// Sent when authorization is required but no token was provided.
    #[error("Missing or malformed credentials")]
    MissingCredentials,
    /// Sent when the server fails to create a token.
    #[error("Token creation failed")]
    TokenCreation,
    /// Sent when the user provides an invalid token.
    #[error("Invalid token")]
    InvalidToken,
    /// Sent when the server fails to hash a password.
    #[error("Failed to generate password hash: {0}")]
    PasswordHash(#[from] argon2::password_hash::Error),
}

impl AuthError {
    const fn status_code(&self) -> StatusCode {
        match self {
            Self::MissingCredentials | Self::TokenCreation | Self::InvalidToken | Self::InvalidCredentials => {
                StatusCode::UNAUTHORIZED
            }
            Self::PasswordHash(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        ErrResponse::new(self.status_code(), self.to_string()).into_response()
    }
}

/// Errors that can occur during either the gateway or REST API execution.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum AppError {
    #[error("Database transaction failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("S3 service returned error: {0}")]
    S3(String),
    #[error("Failed to serialize/deserialize JSON: {0}")]
    JSON(#[from] serde_json::Error),
    #[error("Failed to parse multipart/form-data: {0}")]
    Multipart(#[from] MultipartError),
    #[error("Failed to validate token: {0}")]
    JWT(#[from] jsonwebtoken::errors::Error),
    #[error("Failed to match regex: {0}")]
    Regex(#[from] regex::Error),
    #[error("Failed to build object: {0}")]
    Build(#[from] BuildError),
    #[error("Failed to parse integer: {0}")]
    ParseInt(#[from] ParseIntError),
    #[error("Authentication failure: {0}")]
    Auth(#[from] AuthError),
    #[error("Internal Server Error: {0}")]
    Axum(#[from] axum::Error),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Bad Request: {0}")]
    IllegalArgument(String),
    #[error("Messaging Service Error: {0}")]
    Firebase(#[from] FirebaseError),
    #[error("Messaging Service Error: {0:?}")]
    FirebaseMulti(Vec<FirebaseError>),
    #[error("Internal Server Error: {0}")]
    Unexpected(String),
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Multipart(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::JWT(e) => {
                if matches!(e.kind(), ErrorKind::ExpiredSignature) {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::FORBIDDEN
                }
            }
            Self::Regex(_) | Self::ParseInt(_) | Self::JSON(_) | Self::IllegalArgument(_) => StatusCode::BAD_REQUEST,
            Self::Build(e) => e.status_code(),
            Self::Axum(_)
            | Self::Database(_)
            | Self::S3(_)
            | Self::Firebase(_)
            | Self::FirebaseMulti(_)
            | Self::Unexpected(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Auth(e) => e.status_code(),
            Self::NotFound(_) => StatusCode::NOT_FOUND,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self);
        }
        ErrResponse::new(status, self.to_string()).into_response()
    }
}

/// Hacky workaround for `SdkError` having a generic type parameter
impl<E, R> From<SdkError<E, R>> for AppError
where
    E: std::error::Error + Send + Sync + 'static,
    R: std::fmt::Debug,
{
    fn from(e: SdkError<E, R>) -> Self {
        Self::S3(DisplayErrorContext(e).to_string())
    }
}

/// Errors that can occur during the gateway execution.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GatewayError {
    #[error(transparent)]
    App(AppError),
    #[error("Internal Server Error: {0}")]
    InternalServerError(String),
    #[error("Policy Violation: {0}")]
    PolicyViolation(String),
    #[error("Malformed Frame: {0}")]
    MalformedFrame(String),
    #[error("Authentication Error: {0}")]
    AuthError(String),
    #[error("Handshake Failure: {0}")]
    HandshakeFailure(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
}

// Anything that can be converted into an AppError can be converted into a GatewayError
impl<T: Into<AppError>> From<T> for GatewayError {
    fn from(e: T) -> Self {
        Self::App(e.into())
    }
}

impl GatewayError {
    pub fn close_code(&self) -> GatewayCloseCode {
        match self {
            Self::App(app_error) => app_error.status_code().into(),
            Self::InternalServerError(_) => GatewayCloseCode::ServerError,
            Self::PolicyViolation(_) | Self::AuthError(_) | Self::HandshakeFailure(_) => {
                GatewayCloseCode::PolicyViolation
            }
            Self::MalformedFrame(_) => GatewayCloseCode::InvalidPayload,
            Self::Forbidden(_) => GatewayCloseCode::PolicyViolation,
        }
    }
}

/// Errors that can occur during the REST API execution.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RESTError {
    #[error(transparent)]
    App(AppError),
    #[error("Internal server error: {0}")]
    InternalServerError(String),
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Malformed field: {0}")]
    MalformedField(String),
    #[error("Duplicate field: {0}")]
    DuplicateField(String),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Bad Request: {0}")]
    BadRequest(String),
    #[error("Payload Too Large: {0}")]
    PayloadTooLarge(String),
    #[error("Conflict: {0}")]
    Conflict(String),
}

impl RESTError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::App(e) => e.status_code(),
            Self::InternalServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::MissingField(_) | Self::MalformedField(_) | Self::DuplicateField(_) | Self::BadRequest(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
        }
    }
}

// Anything that can be converted into an AppError can be converted into a RESTError
impl<T: Into<AppError>> From<T> for RESTError {
    fn from(e: T) -> Self {
        Self::App(e.into())
    }
}

impl IntoResponse for RESTError {
    fn into_response(self) -> Response {
        ErrResponse::new(self.status_code(), self.to_string()).into_response()
    }
}
