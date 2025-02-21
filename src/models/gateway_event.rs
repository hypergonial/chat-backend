use futures::future::join_all;
use secrecy::Secret;
use serde::{Deserialize, Serialize};

use super::{
    channel::Channel,
    errors::AppError,
    guild::Guild,
    member::Member,
    message::Message,
    snowflake::Snowflake,
    state::ApplicationState,
    user::{Presence, User},
};

/// A JSON payload that can be received over the websocket by clients.
/// All events are serialized in a way such that they are wrapped in a "data" field.
#[derive(Serialize, Debug, Clone)]
#[non_exhaustive]
#[serde(tag = "event", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GatewayEvent {
    /// The initial message sent on connection.
    Hello { heartbeat_interval: u64 },
    /// A heartbeat acknowledgement.
    HeartbeatAck,
    /// A chat message.
    MessageCreate(Message),
    /// A chat message was updated.
    MessageUpdate(Message),
    /// A chat message was deleted.
    MessageRemove {
        id: Snowflake<Message>,
        channel_id: Snowflake<Channel>,
        guild_id: Option<Snowflake<Guild>>,
    },
    /// A peer has joined the chat.
    MemberCreate(Member),
    /// A peer has left the chat.
    MemberRemove {
        id: Snowflake<User>,
        guild_id: Snowflake<Guild>,
    },
    /// A guild was created.
    GuildCreate(GuildCreatePayload),
    /// A guild was updated.
    GuildUpdate(Guild),
    /// A guild was deleted.
    GuildRemove(Guild),
    /// A channel was created.
    ChannelCreate(Channel),
    /// A channel was deleted.
    ChannelRemove(Channel),
    /// A user's presence was updated.
    PresenceUpdate {
        user_id: Snowflake<User>,
        presence: Presence,
    },
    /// A user has started typing in a channel.
    TypingStart {
        user_id: Snowflake<User>,
        channel_id: Snowflake<Channel>,
    },
    /// The server is ready to accept messages.
    Ready { user: User, guilds: Vec<Guild> },
    /// A user's data was updated.
    UserUpdate(User),
}

/// A JSON payload that can be sent over the websocket by clients.
#[derive(Deserialize, Debug, Clone)]
#[non_exhaustive]
#[serde(tag = "event", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GatewayMessage {
    /// Identify with the server. This should be the first event sent by the client.
    Identify {
        /// The token to authenticate with.
        token: Secret<String>,
    },
    /// A heartbeat message to indicate that the client is still active.
    Heartbeat,
    /// A message to start typing in a channel.
    StartTyping {
        /// The channel to start typing in.
        channel_id: Snowflake<Channel>,
    },
}

/// Represents a `GUILD_CREATE` payload.
///
/// This event is dispatched when a new guild is created, or when initially connecting to the gateway to fill client cache.
#[derive(Serialize, Debug, Clone)]
pub struct GuildCreatePayload {
    pub guild: Guild,
    pub members: Vec<Member>,
    pub channels: Vec<Channel>,
}

impl GuildCreatePayload {
    pub const fn new(guild: Guild, members: Vec<Member>, channels: Vec<Channel>) -> Self {
        Self {
            guild,
            members,
            channels,
        }
    }

    /// Create a new guild create payload by fetching all relevant data from the database.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn from_guild(app: &ApplicationState, guild: Guild) -> Result<Self, AppError> {
        // Presences need to be included in the payload
        let members = join_all(
            app.ops()
                .fetch_members_for(&guild)
                .await?
                .into_iter()
                .map(|m| m.include_presence(app.gateway())),
        )
        .await;

        let channels = app.ops().fetch_channels_for(&guild).await?;
        Ok(Self::new(guild, members, channels))
    }
}
