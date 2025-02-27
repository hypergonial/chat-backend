use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use tower_http::limit::RequestBodyLimitLayer;

use crate::{
    gateway::handler::SendMode,
    models::{
        auth::Token,
        channel::{Channel, ChannelLike},
        errors::RESTError,
        gateway_event::GatewayEvent,
        member::UserLike,
        message::Message,
        requests::UpdateMessage,
        snowflake::Snowflake,
        state::App,
    },
};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FetchMessagesQuery {
    limit: Option<u32>,
    before: Option<Snowflake<Message>>,
    after: Option<Snowflake<Message>>,
    around: Option<Snowflake<Message>>,
}

/* let message_create_lim: SharedIDLimiter = Arc::new(RateLimiter::keyed(
    Quota::per_second(nonzero!(5u32)).allow_burst(nonzero!(5u32)),
)); */

pub fn get_router() -> Router<App> {
    Router::new()
        .route("/channels/{channel_id}", get(fetch_channel))
        .route("/channels/{channel_id}", delete(delete_channel))
        .route("/channels/{channel_id}/messages", post(create_message))
        .route("/channels/{channel_id}/messages", get(fetch_messages))
        .route("/channels/{channel_id}/messages/{message_id}", patch(update_message))
        .route("/channels/{channel_id}/messages/{message_id}", delete(delete_message))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(8 * 1024 * 1024 /* 8mb */))
}

/// Fetch a channel's data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `channel_id` - The ID of the channel to fetch
///
/// ## Returns
///
/// * [`Channel`] - A JSON response containing the fetched [`Channel`] object
///
/// ## Endpoint
///
/// GET `/channels/{channel_id}`
async fn fetch_channel(
    Path(channel_id): Path<Snowflake<Channel>>,
    State(app): State<App>,
    token: Token,
) -> Result<Json<Channel>, RESTError> {
    let channel = app.ops().fetch_channel(channel_id).await.ok_or(RESTError::NotFound(
        "Channel does not exist or is not available.".to_string(),
    ))?;

    // Check if the user is in the channel's guild
    app.ops()
        .fetch_member(token.data().user_id(), channel.guild_id())
        .await?
        .ok_or(RESTError::Forbidden("Not permitted to view resource.".to_string()))?;

    Ok(Json(channel))
}

/// Delete a channel.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `channel_id` - The ID of the channel to delete
///
/// ## Returns
///
/// * [`StatusCode`] - 204 No Content if successful
///
/// ## Dispatches
///
/// * [`GatewayEvent::ChannelRemove`] - To all members who can view the channel
///
/// ## Endpoint
///
/// DELETE `/channels/{channel_id}`
async fn delete_channel(
    Path(channel_id): Path<Snowflake<Channel>>,
    State(app): State<App>,
    token: Token,
) -> Result<StatusCode, RESTError> {
    let channel = app.ops().fetch_channel(channel_id).await.ok_or(RESTError::NotFound(
        "Channel does not exist or is not available.".into(),
    ))?;

    // Check guild owner_id
    let guild = app
        .ops()
        .fetch_guild(channel.guild_id())
        .await
        .ok_or(RESTError::NotFound("Guild does not exist or is not available.".into()))?;

    if guild.owner_id() != token.data().user_id() {
        return Err(RESTError::NotFound("Not permitted to delete channel.".into()));
    }

    app.ops().delete_channel(&channel).await?;

    app.gateway()
        .dispatch(GatewayEvent::ChannelRemove(channel), SendMode::ToGuild(guild.id()));

    Ok(StatusCode::NO_CONTENT)
}

/// Send a new message and return the message data.
///
/// ## Arguments
///
/// * `token` - The authorization token
/// * `payload` - The multipart form data
///
/// ## Returns
///
/// * [`Message`] - A JSON response containing a [`Message`] object
///
/// ## Dispatches
///
/// * [`GatewayEvent::MessageCreate`] - To all members who can view the channel
///
/// ## Endpoint
///
/// POST `/channels/{channel_id}/messages`
async fn create_message(
    Path(channel_id): Path<Snowflake<Channel>>,
    State(app): State<App>,
    token: Token,
    payload: Multipart,
) -> Result<(StatusCode, Json<Message>), RESTError> {
    let channel = app.ops().fetch_channel(channel_id).await.ok_or(RESTError::NotFound(
        "Channel does not exist or is not available.".into(),
    ))?;

    let member = app
        .ops()
        .fetch_member(token.data().user_id(), channel.guild_id())
        .await?
        .ok_or(RESTError::Forbidden("Not permitted to access resource.".into()))?;

    let message = Message::from_formdata(&app.config, UserLike::Member(member), channel_id, payload).await?;

    app.ops().commit_message(&message).await?;

    let message = message.strip_attachment_contents();
    let reply = Json(message.clone());

    app.gateway().dispatch(
        GatewayEvent::MessageCreate(message),
        SendMode::ToGuild(channel.guild_id()),
    );
    Ok((StatusCode::CREATED, reply))
}

/// Update a message.
///
/// ## Arguments
///
/// * `channel_id` - The ID of the channel the message is in
/// * `message_id` - The ID of the message to update
/// * `token` - The authorization token
/// * `payload` - The update payload
///
/// ## Returns
///
/// * [`Message`] - A JSON response containing the updated [`Message`] object
///
/// ## Endpoint
///
/// PATCH `/channels/{channel_id}/messages/{message_id}`
async fn update_message(
    Path((channel_id, message_id)): Path<(Snowflake<Channel>, Snowflake<Message>)>,
    State(app): State<App>,
    token: Token,
    Json(payload): Json<UpdateMessage>,
) -> Result<Json<Message>, RESTError> {
    let message = app.ops().fetch_message(message_id).await?.ok_or(RESTError::NotFound(
        "Message does not exist or is not available.".into(),
    ))?;

    if message.channel_id() != channel_id {
        return Err(RESTError::NotFound(
            "Message does not exist or is not available.".into(),
        ));
    }

    let channel = app.ops().fetch_channel(channel_id).await.ok_or(RESTError::NotFound(
        "Channel does not exist or is not available.".into(),
    ))?;

    let member = app
        .ops()
        .fetch_member(token.data().user_id(), channel.guild_id())
        .await?
        .ok_or(RESTError::Forbidden("Not permitted to access resource.".into()))?;

    if member.user().id() != message.author().map_or(Snowflake::new(0), UserLike::id) {
        return Err(RESTError::Forbidden("Not permitted to patch resource.".into()));
    }

    let msg = payload.perform_request(&app, message_id).await?;

    let reply = Json(msg.clone());
    app.gateway()
        .dispatch(GatewayEvent::MessageUpdate(msg), SendMode::ToGuild(channel.guild_id()));

    Ok(reply)
}

/// Delete a message.
///
/// ## Arguments
///
/// * `channel_id` - The ID of the channel the message is in
/// * `message_id` - The ID of the message to delete
/// * `token` - The authorization token
///
/// ## Returns
///
/// * [`StatusCode`] - 204 No Content if successful
///
/// ## Dispatches
///
/// * [`GatewayEvent::MessageDelete`] - To all members who can view the channel
///
/// ## Endpoint
///
/// DELETE `/channels/{channel_id}/messages/{message_id}`
async fn delete_message(
    Path((channel_id, message_id)): Path<(Snowflake<Channel>, Snowflake<Message>)>,
    State(app): State<App>,
    token: Token,
) -> Result<StatusCode, RESTError> {
    let message = app.ops().fetch_message(message_id).await?.ok_or(RESTError::NotFound(
        "Message does not exist or is not available.".into(),
    ))?;

    let channel = app.ops().fetch_channel(channel_id).await.ok_or(RESTError::NotFound(
        "Channel does not exist or is not available.".into(),
    ))?;

    if message.channel_id() != channel_id {
        return Err(RESTError::NotFound(
            "Message does not exist or is not available.".into(),
        ));
    }

    let member = app
        .ops()
        .fetch_member(token.data().user_id(), channel.guild_id())
        .await?
        .ok_or(RESTError::Forbidden("Not permitted to access resource.".into()))?;

    if member.user().id() != message.author().map_or(Snowflake::new(0), UserLike::id) {
        return Err(RESTError::Forbidden("Not permitted to delete resource.".into()));
    }

    app.ops().delete_message(channel_id, message).await?;

    app.gateway().dispatch(
        GatewayEvent::MessageRemove {
            id: message_id,
            channel_id,
            guild_id: Some(channel.guild_id()),
        },
        SendMode::ToGuild(channel.guild_id()),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Fetch a channel's messages.
///
/// ## Arguments
///
/// * `token` - The authorization token
/// * `channel_id` - The ID of the channel to fetch messages from
/// * `query` - The query parameters
///
/// ## Returns
///
/// * [`Vec<Message>`] - A JSON response containing a list of [`Message`] objects
///
/// ## Endpoint
///
/// GET `/channels/{channel_id}/messages`
async fn fetch_messages(
    Path(channel_id): Path<Snowflake<Channel>>,
    State(app): State<App>,
    token: Token,
    Query(query): Query<FetchMessagesQuery>,
) -> Result<(StatusCode, Json<Vec<Message>>), RESTError> {
    let channel = app.ops().fetch_channel(channel_id).await.ok_or(RESTError::NotFound(
        "Channel does not exist or is not available.".into(),
    ))?;

    // Check if the user is in the channel's guild
    app.ops()
        .fetch_member(token.data().user_id(), channel.guild_id())
        .await?
        .ok_or(RESTError::Forbidden("Not permitted to view resource.".into()))?;

    let messages = app
        .ops()
        .fetch_messages_from(channel_id, query.limit, query.before, query.after, query.around)
        .await?;

    Ok((StatusCode::OK, Json(messages)))
}
