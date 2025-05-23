use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};

use crate::{
    app::App,
    gateway::SendMode,
    models::{
        auth::Token,
        channel::Channel,
        errors::RESTError,
        gateway_event::{GatewayEvent, GuildCreatePayload},
        guild::Guild,
        member::Member,
        request_payloads::{CreateChannel, CreateGuild, UpdateGuild},
        snowflake::Snowflake,
        user::User,
    },
};

pub fn get_router() -> Router<App> {
    Router::new()
        .route("/guilds", post(create_guild))
        .route("/guilds/{guild_id}", get(fetch_guild))
        .route("/guilds/{guild_id}/channels", post(create_channel))
        .route("/guilds/{guild_id}/members", post(create_member))
        .route("/guilds/{guild_id}/members/@me", get(fetch_member_self))
        .route("/guilds/{guild_id}/members/{member_id}", get(fetch_member))
        .route("/guilds/{guild_id}/members/@me", delete(leave_guild))
        .route("/guilds/{guild_id}", delete(delete_guild))
        .route(
            "/guilds/{guild_id}",
            patch(update_guild).layer(DefaultBodyLimit::max(3 * 1024 * 1024 /* 3mb */)),
        )
}

/// Create a new guild and return the guild data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `payload` - The [`CreateGuild`] payload, containing the guild name
///
/// ## Returns
///
/// * [`Guild`] - A JSON response containing the created [`Guild`] object
///
/// ## Dispatches
///
/// * [`GatewayEvent::GuildCreate`] - Dispatched when the guild is created
///
/// ## Endpoint
///
/// POST `/guilds`
async fn create_guild(
    token: Token,
    State(app): State<App>,
    Json(payload): Json<CreateGuild>,
) -> Result<(StatusCode, Json<Guild>), RESTError> {
    let (guild, general, owner) = payload.perform_request(&app, token.data().user_id()).await?;

    app.gateway().add_member(token.data().user_id(), &guild);

    app.gateway().dispatch(
        GatewayEvent::GuildCreate(GuildCreatePayload::new(guild.clone(), vec![owner], vec![general])),
        SendMode::ToGuild(guild.id()),
    );

    Ok((StatusCode::CREATED, Json(guild)))
}

/// Create a new channel in a guild and return the channel data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `guild_id` - The ID of the guild to create the channel in
/// * `payload` - The [`CreateChannel`] payload, containing the channel name
///
/// ## Returns
///
/// * [`Channel`] - A JSON response containing the created [`Channel`] object
///
/// ## Dispatches
///
/// * [`GatewayEvent::ChannelCreate`] - To all guild members
///
/// ## Endpoint
///
/// POST `/guilds/{guild_id}/channels`
async fn create_channel(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
    Json(payload): Json<CreateChannel>,
) -> Result<(StatusCode, Json<Channel>), RESTError> {
    let guild = app
        .ops()
        .fetch_guild(guild_id)
        .await
        .ok_or(RESTError::NotFound("Guild not found".into()))?;

    if guild.owner_id() != token.data().user_id() {
        return Err(RESTError::Forbidden("You are not the owner of this guild.".into()));
    }

    let channel = Channel::from_payload(&app.config, payload, guild_id);

    app.ops().create_channel(&channel).await?;

    app.gateway().dispatch(
        GatewayEvent::ChannelCreate(channel.clone()),
        SendMode::ToGuild(guild_id),
    );

    Ok((StatusCode::CREATED, Json(channel)))
}

/// Fetch a guild's data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `guild_id` - The ID of the guild to fetch
///
/// ## Returns
///
/// * [`Guild`] - A JSON response containing the fetched [`Guild`] object
///
/// ## Endpoint
///
/// GET `/guilds/{guild_id}`
async fn fetch_guild(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
) -> Result<Json<Guild>, RESTError> {
    app.ops()
        .fetch_member(token.data().user_id(), guild_id)
        .await?
        .ok_or(RESTError::Forbidden("Not permitted to view resource.".into()))?;

    let guild = app
        .ops()
        .fetch_guild(guild_id)
        .await
        .ok_or(RESTError::InternalServerError(
            "Failed to fetch guild from database".into(),
        ))?;

    Ok(Json(guild))
}

/// Update a guild's data.
///
/// ## Arguments
///
/// * `guild_id` - The ID of the guild to update
/// * `token` - The user's session token, already validated
/// * `payload` - The [`UpdateGuild`] payload, containing the fields to update
///
/// ## Returns
///
/// * [`Guild`] - A JSON response containing the updated [`Guild`] object
///
/// ## Endpoint
///
/// PATCH `/guilds/{guild_id}`
async fn update_guild(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
    Json(payload): Json<UpdateGuild>,
) -> Result<Json<Guild>, RESTError> {
    let guild = app
        .ops()
        .fetch_guild(guild_id)
        .await
        .ok_or(RESTError::NotFound("Guild does not exist or is not available.".into()))?;

    if guild.owner_id() != token.data().user_id() {
        return Err(RESTError::Forbidden("Not permitted to update resource.".into()));
    }
    let guild = payload.perform_request(&app, &guild).await?;

    app.gateway()
        .dispatch(GatewayEvent::GuildUpdate(guild.clone()), SendMode::ToGuild(guild.id()));

    Ok(Json(guild))
}

/// Delete a guild and all associated objects
///
/// ## Arguments
///
/// * `guild_id` - The ID of the guild to delete
/// * `token` - The user's session token, already validated
///
/// ## Endpoint
///
/// DELETE `/guilds/{guild_id}`
async fn delete_guild(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
) -> Result<StatusCode, RESTError> {
    let guild = app
        .ops()
        .fetch_guild(guild_id)
        .await
        .ok_or(RESTError::NotFound("Guild does not exist or is not available.".into()))?;

    if guild.owner_id() != token.data().user_id() {
        return Err(RESTError::Forbidden("Not permitted to delete guild.".into()));
    }

    app.ops().delete_guild(&guild).await?;

    app.gateway()
        .dispatch(GatewayEvent::GuildRemove(guild.clone()), SendMode::ToGuild(guild_id));

    Ok(StatusCode::NO_CONTENT)
}

/// Fetch a member's data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `guild_id` - The ID of the guild the member is in
///
/// ## Returns
///
/// * [`Member`] - A JSON response containing the fetched [`Member`] object
///
/// ## Endpoint
///
/// GET `/guilds/{guild_id}/members/{member_id}`
async fn fetch_member(
    Path(guild_id): Path<Snowflake<Guild>>,
    Path(member_id): Path<Snowflake<User>>,
    State(app): State<App>,
    token: Token,
) -> Result<Json<Member>, RESTError> {
    // Check if the requester is in the channel's guild
    if !app.ops().has_member(guild_id, token.data().user_id()).await? {
        return Err(RESTError::Forbidden("Not permitted to view resource.".into()));
    }

    let member = app
        .ops()
        .fetch_member(member_id, guild_id)
        .await?
        .ok_or(RESTError::NotFound("Member does not exist or is not available.".into()))?;

    Ok(Json(member))
}

/// Fetch the current user's member data.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `guild_id` - The ID of the guild the member is in
///
/// ## Returns
///
/// * [`Member`] - A JSON response containing the fetched [`Member`] object
///
/// ## Endpoint
///
/// GET `/guilds/{guild_id}/members/@me`
async fn fetch_member_self(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
) -> Result<Json<Member>, RESTError> {
    let member = app
        .ops()
        .fetch_member(token.data().user_id(), guild_id)
        .await?
        .ok_or(RESTError::NotFound("Member does not exist or is not available.".into()))?;

    Ok(Json(member))
}

/// Add the token-holder to a guild.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `guild_id` - The ID of the guild to add the user to
///
/// ## Returns
///
/// * [`Member`] - A JSON response containing the created [`Member`] object
///
/// ## Dispatches
///
/// * [`GatewayEvent::GuildCreate`] - For the user who joined the guild
/// * [`GatewayEvent::MemberCreate`] - For all members already in the guild
///
/// ## Endpoint
///
/// POST `/guilds/{guild_id}/members`
async fn create_member(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
) -> Result<(StatusCode, Json<Member>), RESTError> {
    let guild = app
        .ops()
        .fetch_guild(guild_id)
        .await
        .ok_or(RESTError::NotFound("Guild does not exist or is not available.".into()))?;

    let member = app.ops().create_member(&guild, token.data().user_id()).await?;

    // Create payload seperately as it needs read access to gateway
    let gc_payload = GatewayEvent::GuildCreate(GuildCreatePayload::from_guild(&app, guild).await?);

    // Send GUILD_CREATE to the user who joined
    app.gateway().send_to(&member, gc_payload);

    // Add the member to the gateway's cache
    app.gateway().add_member(&member, guild_id);

    // Dispatch the member create event to all guild members
    app.gateway()
        .dispatch(GatewayEvent::MemberCreate(member.clone()), SendMode::ToGuild(guild_id));

    Ok((StatusCode::CREATED, Json(member)))
}

/// Remove the token-holder from a guild.
///
/// ## Arguments
///
/// * `token` - The user's session token, already validated
/// * `guild_id` - The ID of the guild to remove the user from
///
/// ## Returns
///
/// * `()` - An empty response
///
/// ## Dispatches
///
/// * [`GatewayEvent::GuildRemove`] - For the user who left the guild
/// * [`GatewayEvent::MemberRemove`] - For all members still in the guild
///
/// ## Endpoint
///
/// DELETE `/guilds/{guild_id}/members/@me`
async fn leave_guild(
    Path(guild_id): Path<Snowflake<Guild>>,
    State(app): State<App>,
    token: Token,
) -> Result<StatusCode, RESTError> {
    let guild = app
        .ops()
        .fetch_guild(guild_id)
        .await
        .ok_or(RESTError::NotFound("Guild does not exist or is not available.".into()))?;
    let member_id = token.data().user_id();

    if !app.ops().has_member(guild_id, member_id).await? {
        return Err(RESTError::NotFound("Member does not exist.".into()));
    }

    if member_id == guild.owner_id() {
        return Err(RESTError::Forbidden("Owner cannot leave owned guild.".into()));
    }

    app.ops().delete_member(&guild, member_id).await?;

    // Remove the member from the gateway's sessions
    app.gateway().remove_member(member_id, guild_id);

    // Send GUILD_REMOVE to the user who left
    app.gateway().send_to(member_id, GatewayEvent::GuildRemove(guild));

    // Dispatch the member remove event
    app.gateway().dispatch(
        GatewayEvent::MemberRemove {
            id: member_id,
            guild_id,
        },
        SendMode::ToGuild(guild_id),
    );

    Ok(StatusCode::NO_CONTENT)
}
