use std::{
    collections::HashMap,
    fmt::{self, Debug, Display, Formatter},
    num::NonZero,
    sync::Arc,
    time::Duration,
};

use gcp_auth;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

/// See: <https://firebase.google.com/docs/cloud-messaging/auth-server#use-credentials-to-mint-access-tokens>
static FCM_SCOPES: [&str; 1] = ["https://www.googleapis.com/auth/firebase.messaging"];

#[derive(Serialize)]
struct FirebaseMessage<'a> {
    token: &'a str,
    notification: &'a Notification,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a HashMap<String, String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum FCMErrorCode {
    UnspecifiedError,
    InvalidArgument,
    Unregistered,
    SenderIdMismatch,
    QuotaExceeded,
    Unavailable,
    Internal,
    ThirdPartyAuthError,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Error, Deserialize)]
pub struct GCPError {
    error: GCPErrorInner,
}

impl GCPError {
    /// The HTTP status code of the error.
    pub const fn code(&self) -> NonZero<u16> {
        self.error.code
    }

    /// The error message.
    pub fn message(&self) -> &str {
        &self.error.message
    }

    /// The status message of the error.
    pub fn status(&self) -> &str {
        &self.error.status
    }

    /// The error details.
    ///
    /// This can be used to determine the specific error(s) that occurred.
    pub fn details(&self) -> &[GCPErrorDetail] {
        &self.error.details
    }

    /// Returns the FCM error code if the error is an FCM error.
    ///
    /// This is a convenience method that filters the error details for an FCM error code.
    ///
    /// # Returns
    ///
    /// Returns the FCM error code if the error is an FCM error, otherwise `None`.
    pub fn get_fcm_error_code(&self) -> Option<FCMErrorCode> {
        self.error.details.iter().find_map(|detail| match detail {
            GCPErrorDetail::FcmError { error_code } => Some(*error_code),
            _ => None,
        })
    }
}

impl Display for GCPError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)
    }
}

#[derive(Debug, Clone, Error, Deserialize)]
struct GCPErrorInner {
    code: NonZero<u16>,
    message: String,
    status: String,
    details: Vec<GCPErrorDetail>,
}

impl Display for GCPErrorInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "GCP error - {}: {}", self.code, self.message)
    }
}

#[derive(Debug, Clone, Error, Deserialize)]
#[serde(tag = "@type")]
#[non_exhaustive]
pub enum GCPErrorDetail {
    #[serde(rename = "type.googleapis.com/google.firebase.fcm.v1.FcmError")]
    FcmError { error_code: FCMErrorCode },
    #[serde(other)]
    Unknown,
}

impl Display for GCPErrorDetail {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FcmError { error_code } => write!(f, "FCM error: {error_code:?}"),
            Self::Unknown => write!(f, "Unknown error"),
        }
    }
}

/// An implementation of Firebase Cloud Messaging (FCM) for sending push notifications.
pub struct FirebaseMessaging {
    provider: Arc<dyn gcp_auth::TokenProvider>,
    project_id: String,
    http: Arc<reqwest::Client>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FirebaseErrorKind {
    #[error("Failed to authenticate with notification service: {0}")]
    Auth(#[from] gcp_auth::Error),
    #[error("Failed to send push notification: {0}")]
    Request(#[from] reqwest::Error),
    #[error("API returned error: {0}")]
    Api(#[from] GCPError),
    #[error("Task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

#[derive(Error)]
pub struct FirebaseError {
    /// The kind of error that occurred
    kind: FirebaseErrorKind,
    /// The request token that caused the error
    token: Option<String>,
}

impl Debug for FirebaseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FirebaseError")
            .field("kind", &self.kind)
            .field("token", &self.token.as_deref().map(|_| "<redacted>"))
            .finish()
    }
}

impl FirebaseError {
    const fn new(kind: FirebaseErrorKind, token: Option<String>) -> Self {
        Self { kind, token }
    }

    /// The type of error that occurred
    pub const fn kind(&self) -> &FirebaseErrorKind {
        &self.kind
    }

    /// The request token that belongs to the error
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}

impl Display for FirebaseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl FirebaseMessaging {
    /// Create a new instance of the FCM client.
    ///
    /// # Panics
    ///
    /// If the TLS backend cannot be initialized,
    /// or the HTTP client resolver cannot load the system configuration.
    ///
    /// # Errors
    ///
    /// If no GCP auth provider was resolved.
    /// This is typically due to a missing `GOOGLE_APPLICATION_CREDENTIALS` environment variable.
    pub async fn new() -> Result<Self, FirebaseErrorKind> {
        let provider = gcp_auth::provider().await?;
        let project_id = provider.project_id().await?.to_string();

        Ok(Self {
            provider,
            project_id,
            http: Arc::new(
                reqwest::Client::builder()
                    .use_rustls_tls()
                    .http2_prior_knowledge()
                    .build()
                    .expect("Failed to create HTTP client for FCM"),
            ),
        })
    }

    /// Get a new access token for the FCM API.
    ///
    /// Calling this function multiple times will not result in multiple requests, as
    /// the token is cached and refreshed automatically when it expires.
    ///
    /// # Errors
    ///
    /// If the token cannot be fetched.
    async fn get_token(&self) -> Result<Arc<gcp_auth::Token>, gcp_auth::Error> {
        self.provider.token(&FCM_SCOPES).await
    }

    /// Helper function to determine if an error is retriable
    fn is_retriable_error(err: &reqwest::Error) -> bool {
        if let Some(status) = err.status() {
            if status.is_server_error() {
                return true;
            }

            matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::TOO_EARLY | StatusCode::REQUEST_TIMEOUT
            )
        } else {
            err.is_timeout() || err.is_connect()
        }
    }

    fn is_retriable_status(status: StatusCode) -> bool {
        status.is_server_error()
            || matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::TOO_EARLY | StatusCode::REQUEST_TIMEOUT
            )
    }

    async fn perform_send(
        http: &reqwest::Client,
        auth_token: impl Into<&str>,
        project_id: impl Into<&str>,
        to: impl Into<&str>,
        notification: &Notification,
        data: Option<&HashMap<String, String>>,
    ) -> Result<(), FirebaseError> {
        let auth_token = auth_token.into();
        let project_id = project_id.into();
        let token = to.into();
        let message = FirebaseMessage {
            token,
            notification,
            data,
        };

        let mut attempts = 0;
        let max_attempts = 10; // Maximum retry attempts
        let base_delay = Duration::from_secs(10); // Starting delay

        loop {
            attempts += 1;

            let result = http
                .post(format!(
                    "https://fcm.googleapis.com/v1/projects/{project_id}/messages:send",
                ))
                .header("Authorization", format!("Bearer {auth_token}"))
                .json(&json!({ "message": message }))
                .send()
                .await;

            let mut retry_after = None;

            match result {
                Ok(response) => {
                    let is_error = response.status().is_client_error() || response.status().is_server_error();

                    // If we can't retry, bail
                    if is_error && (attempts >= max_attempts || !Self::is_retriable_status(response.status())) {
                        let body = response
                            .json::<GCPError>()
                            .await
                            .map_err(|e| FirebaseError::new(FirebaseErrorKind::Request(e), Some(token.to_string())))?;
                        return Err(FirebaseError::new(
                            FirebaseErrorKind::Api(body),
                            Some(token.to_string()),
                        ));
                    // If we can retry, try extracting the Retry-After header
                    } else if is_error {
                        retry_after = response
                            .headers()
                            .get("Retry-After")
                            .and_then(|value| value.to_str().ok().and_then(|value| value.parse::<u64>().ok()));

                        if retry_after.is_none() && response.status() == StatusCode::TOO_MANY_REQUESTS {
                            retry_after = Some(60); // Default to 60 seconds as per FCM docs, quotas reset every minute
                        }
                    } else {
                        return Ok(());
                    }
                }
                // Something went wrong with the request itself
                Err(err) => {
                    if attempts >= max_attempts || !Self::is_retriable_error(&err) {
                        return Err(FirebaseError::new(
                            FirebaseErrorKind::Request(err),
                            Some(token.to_string()),
                        ));
                    }
                }
            }

            // Try respecting the Retry-After header, falling back to exponential backoff
            let delay = retry_after.map_or_else(|| base_delay * 2u32.pow(attempts - 1), Duration::from_secs);
            let jitter = rand::random::<f32>() * 0.5; // Add up to 50% jitter
            let delay_with_jitter = delay.mul_f32(1.0 + jitter);

            tracing::debug!(
                "FCM messages:send retry {}/{} after {:?}",
                attempts,
                max_attempts,
                delay_with_jitter
            );

            tokio::time::sleep(delay_with_jitter).await;
        }
    }

    /// Send a push notification to a device.
    ///
    /// # Arguments
    ///
    /// * `token` - The device token to send the notification to
    /// * `notification` - The notification to send
    /// * `data` - Additional data to send with the notification
    ///
    /// # Errors
    ///
    /// If the notification could not be sent.
    pub async fn send_notification(
        &self,
        token: impl Into<String>,
        notification: Notification,
        data: Option<HashMap<String, String>>,
    ) -> Result<(), FirebaseError> {
        let token = token.into();

        Self::perform_send(
            &self.http,
            self.get_token()
                .await
                .map_err(|e| FirebaseError::new(FirebaseErrorKind::Auth(e), Some(token.clone())))?
                .as_str(),
            &*self.project_id,
            &*token,
            &notification,
            data.as_ref(),
        )
        .await?;

        Ok(())
    }

    /// Send a push notification to multiple devices.
    ///
    /// This function will send notifications to each device in parallel.
    ///
    /// # Arguments
    ///
    /// * `tokens` - A list of device tokens to send the notification to
    /// * `notification` - The notification to send
    /// * `data` - Additional data to send with the notification
    ///
    /// # Panics
    ///
    /// If any of the underlying send tasks panics. (This should not happen)
    ///
    /// # Errors
    ///
    /// Returns a list of errors for each token that failed to receive the notification.
    ///
    /// You can use [`FirebaseError::token()`] to get the token that caused the error, if any.
    pub async fn send_notification_to_multiple(
        &self,
        tokens: impl IntoIterator<Item = impl Into<String>>,
        notification: Notification,
        data: Option<HashMap<String, String>>,
    ) -> Result<(), Vec<FirebaseError>> {
        let mut peekable = tokens.into_iter().peekable();

        if peekable.peek().is_none() {
            return Ok(());
        }

        // Spawn n tasks for each token
        let auth_token: Arc<str> = Arc::from(
            self.get_token()
                .await
                .map_err(|e| vec![FirebaseError::new(FirebaseErrorKind::Auth(e), None)])?
                .as_str(),
        );
        let project_id: Arc<str> = Arc::from(self.project_id.clone());
        let notification: Arc<Notification> = Arc::new(notification);
        let data: Option<Arc<HashMap<String, String>>> = data.map(Arc::new);

        let tasks = peekable.map(|token| {
            let auth_token = auth_token.clone();
            let project_id = project_id.clone();
            let notification = notification.clone();
            let data = data.clone();
            let token = token.into();
            let http = self.http.clone();

            tokio::spawn(async move {
                Self::perform_send(
                    &http,
                    &*auth_token,
                    &*project_id,
                    &*token,
                    &notification,
                    data.as_deref(),
                )
                .await
            })
        });

        // Wait for all tasks to complete
        let errors = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|r| match r {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) => Err(err),
                Err(err) => {
                    assert!(!err.is_panic(), "Task panicked: {err:?}");
                    Err(FirebaseError::new(FirebaseErrorKind::Task(err), None))
                }
            })
            .filter_map(Result::err)
            .collect::<Vec<FirebaseError>>();

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}
