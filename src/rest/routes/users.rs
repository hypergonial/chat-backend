use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use tower_http::limit::RequestBodyLimitLayer;

use crate::{
    app::App,
    gateway::SendMode,
    models::{
        auth::{Credentials, StoredCredentials, Token},
        errors::RESTError,
        gateway_event::GatewayEvent,
        guild::Guild,
        request_payloads::CreateUser,
        request_payloads::UpdateUser,
        user::{Presence, User},
    },
    rest::auth::{generate_hash, validate_credentials},
};

pub fn get_router() -> Router<App> {
    Router::new()
        .route("/users", post(create_user))
        .route("/users/auth", get(auth_user))
        .route("/users/@me", get(fetch_self))
        .route("/users/@me/guilds", get(fetch_self_guilds))
        .route("/users/@me/presence", patch(update_presence))
        .route("/usernames/{username}", get(query_username))
        .route(
            "/users/@me",
            patch(update_self).layer(RequestBodyLimitLayer::new(2 * 1024 * 1024 /* 2mb */)),
        )
}

/// Create a new user and return the user data.
///
/// ## Arguments
///
/// * `payload` - The `CreateUser` payload, containing the username and password
///
/// ## Returns
///
/// * [`User`] - A JSON response containing the created [`User`] object
///
/// ## Endpoint
///
/// POST `/users`
async fn create_user(State(app): State<App>, Json(payload): Json<CreateUser>) -> Result<Json<User>, RESTError> {
    let password = payload.password.clone();

    if app.ops().is_username_taken(&payload.username).await? {
        return Err(RESTError::BadRequest(format!(
            "User with username {} already exists",
            payload.username
        )));
    }

    // User needs to be created before credentials to avoid foreign key constraint violation
    let user = app.ops().create_user(payload).await?;
    let credentials = StoredCredentials::new(user.id(), generate_hash(&password)?);
    credentials.commit(app).await?;

    Ok(Json(user))
}

/// Validate a user's credentials and return a token if successful.
///
/// ## Arguments
///
/// * `credentials` - The user's credentials
///
/// ## Returns
///
/// * `{"user_id": user_id, "token": token}` - A JSON response containing the session token and `user_id`
///
/// ## Endpoint
///
/// GET `/users/auth`
async fn auth_user(State(app): State<App>, credentials: Credentials) -> Result<Json<Value>, RESTError> {
    let user_id = validate_credentials(app.clone(), credentials).await?;
    let token = Token::new_for(app.config.app_secret(), user_id)?;

    Ok(Json(json!({
        "user_id": user_id,
        "token": token.expose_secret(),
    })))
}

/// Get the current user's data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
///
/// ## Returns
///
/// * [`User`] - A JSON response containing the user's data
///
/// ## Endpoint
///
/// GET `/users/@me`
async fn fetch_self(State(app): State<App>, token: Token) -> Result<Json<User>, RESTError> {
    app.ops()
        .fetch_user(token.data().user_id())
        .await
        .ok_or(RESTError::NotFound("User not found".into()))
        .map(Json)
}

/// Fetch a user's guilds.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
///
/// ## Returns
///
/// * [`Vec<Guild>`] - A JSON response containing the fetched [`Guild`] objects
///
/// ## Endpoint
///
/// GET `/users/@me/guilds`
async fn fetch_self_guilds(State(app): State<App>, token: Token) -> Result<Json<Vec<Guild>>, RESTError> {
    let guilds = app.ops().fetch_guilds_for(token.data().user_id()).await?;

    Ok(Json(guilds))
}

/// Update the token-holder's presence.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `new_presence` - The new presence to set
///
/// ## Returns
///
/// * [`Presence`] - A JSON response containing the updated [`Presence`] object
///
/// ## Errors
///
/// * [`RESTError::NotFound`] - If the user is not found
/// * [`RESTError::App`] - If the database query fails
///
/// ## Dispatches
///
/// * [`GatewayEvent::PresenceUpdate`] - For all members in guilds shared with the user
///
/// ## Endpoint
///
/// PATCH `/users/@me/presence`
// TODO: Remove this endpoint in favour of sending presence updates over the gateway
pub async fn update_presence(
    State(app): State<App>,
    token: Token,
    Json(new_presence): Json<Presence>,
) -> Result<Json<Presence>, RESTError> {
    let user_id_i64: i64 = token.data().user_id().into();

    sqlx::query!(
        "UPDATE users SET last_presence = $1 WHERE id = $2",
        new_presence as i16,
        user_id_i64
    )
    .execute(app.db())
    .await?;

    if app.gateway().is_connected(token.data().user_id()).await {
        app.gateway().dispatch(
            GatewayEvent::PresenceUpdate {
                presence: new_presence,
                user_id: token.data().user_id(),
            },
            SendMode::ToMutualGuilds(token.data().user_id()),
        );
    }

    Ok(Json(new_presence))
}

/// Update the token-holder's user data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `payload` - The `UpdateUser` payload, containing the new user data
///
/// ## Returns
///
/// * [`User`] - A JSON response containing the updated [`User`] object
///
/// ## Errors
///
/// * [`RESTError::NotFound`] - If the user is not found
/// * [`RESTError::App`] - If the database query fails, or the user data is invalid
///
/// ## Endpoint
///
/// PATCH `/users/@me`
pub async fn update_self(
    State(app): State<App>,
    token: Token,
    Json(payload): Json<UpdateUser>,
) -> Result<Json<User>, RESTError> {
    let user = payload.perform_request(&app, token.data().user_id()).await?;
    app.gateway().dispatch(
        GatewayEvent::UserUpdate(user.clone()),
        SendMode::ToMutualGuilds(user.id()),
    );

    Ok(Json(user))
}

/// Check for the existence of a user with the given username.
///
/// ## Arguments
///
/// * `username` - The username to check for
///
/// ## Returns
///
/// * `204 No Content` - If the user exists
///
/// ## Errors
///
/// * [`RESTError::NotFound`] - If the user is not found
/// * [`RESTError::App`] - If the database query fails
///
/// ## Endpoint
///
/// GET `/users/{username}`
pub async fn query_username(State(app): State<App>, Path(username): Path<String>) -> Result<StatusCode, RESTError> {
    if app.ops().is_username_taken(&username).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(RESTError::NotFound("User not found".into()))
    }
}
