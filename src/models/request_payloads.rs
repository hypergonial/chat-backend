use secrecy::Secret;
use serde::Deserialize;

use crate::app::ApplicationState;

use super::{
    channel::Channel,
    data_uri::DataUri,
    errors::{AppError, RESTError},
    guild::Guild,
    member::Member,
    message::Message,
    omittableoption::OmittableOption,
    prefs::{Layout, PrefFlags},
    snowflake::Snowflake,
    user::User,
};

/// A request to create a new user
#[derive(Deserialize, Debug, Clone)]
pub struct CreateUser {
    pub username: String,
    pub password: Secret<String>,
}

/// The JSON part of a multipart form request to create a message
#[derive(Debug, Clone, Deserialize)]
pub struct CreateMessage {
    pub content: Option<String>,
    pub nonce: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct CreateGuild {
    pub name: String,
}

impl CreateGuild {
    /// Perform the create operation
    ///
    /// This is a shorthand for `app.ops().create_guild(payload, owner).await`
    ///
    /// # Parameters
    ///
    /// - `app` - The application state
    /// - `owner` - The owner of the guild
    ///
    /// # Returns
    ///
    /// The created guild, channel, and member
    ///
    /// # Errors
    ///
    /// Fails if the guild creation fails
    #[inline]
    pub async fn perform_request(
        self,
        app: &ApplicationState,
        owner: impl Into<Snowflake<User>>,
    ) -> Result<(Guild, Channel, Member), AppError> {
        app.ops().create_guild(self, owner).await
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct UpdateGuild {
    pub name: Option<String>,
    pub owner_id: Option<Snowflake<User>>,
    #[serde(default)]
    pub avatar: OmittableOption<DataUri>,
}

impl UpdateGuild {
    /// Perform the update operation
    ///
    /// This is a shorthand for `app.ops().update_guild(payload).await`
    ///
    /// # Parameters
    ///
    /// - `app` - The application state
    /// - `guild` - The current guild state that needs to be updated
    ///
    /// # Returns
    ///
    /// The updated guild
    ///
    /// # Errors
    ///
    /// Fails if the guild does not exist or the update operation fails
    #[inline]
    pub async fn perform_request(self, app: &ApplicationState, guild: &Guild) -> Result<Guild, RESTError> {
        app.ops().update_guild(self, guild).await
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreateChannel {
    GuildText { name: String },
}

#[derive(Deserialize, Debug, Clone)]
pub struct UpdateUser {
    pub username: Option<String>,
    #[serde(default)]
    pub display_name: OmittableOption<String>,
    #[serde(default)]
    pub avatar: OmittableOption<DataUri>,
}

impl UpdateUser {
    /// Perform the update operation
    /// This is a shorthand for `app.ops().update_user(user, payload).await`
    ///
    /// # Parameters
    ///
    /// - `app` - The application state
    /// - `user` - The user to update
    ///
    /// # Returns
    ///
    /// The updated user
    ///
    /// # Errors
    ///
    /// Fails if the user does not exist or the update operation fails
    #[inline]
    pub async fn perform_request(
        self,
        app: &ApplicationState,
        user: impl Into<Snowflake<User>>,
    ) -> Result<User, RESTError> {
        app.ops().update_user(user, self).await
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct UpdateMessage {
    #[serde(default)]
    pub content: OmittableOption<String>,
}

impl UpdateMessage {
    /// Perform the update operation
    ///
    /// This is a shorthand for `app.ops().update_message(message, payload).await`
    ///
    /// # Parameters
    ///
    /// - `app` - The application state
    /// - `message` - The message to update
    ///
    /// # Returns
    ///
    /// The updated message
    ///
    /// # Errors
    ///
    /// Fails if the message does not exist or the update operation fails
    #[inline]
    pub async fn perform_request(
        self,
        app: &ApplicationState,
        message: impl Into<Snowflake<Message>>,
    ) -> Result<Message, AppError> {
        app.ops().update_message(message, self).await
    }
}

/// Update payload for user preferences
#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePrefs {
    pub flags: Option<PrefFlags>,
    pub message_grouping_timeout: Option<u64>,
    pub layout: Option<Layout>,
    pub text_size: Option<u8>,
    pub locale: Option<String>,
}

/// Update payload for FCM token updates
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateFCMToken {
    pub token: String,
    pub previous_token: Option<String>,
}

impl UpdateFCMToken {
    /// Perform the update operation
    ///
    /// This is a shorthand for `app.ops().update_fcm_token(user, payload).await`
    ///
    /// # Parameters
    ///
    /// - `app` - The application state
    /// - `user` - The user to update
    ///
    /// # Errors
    ///
    /// - [`AppError::NotFound`] if the user does not exist
    /// - [`AppError::Database`] if the update operation fails
    /// - [`RESTError::Conflict`] if the token already exists
    #[inline]
    pub async fn perform_request(
        self,
        app: &ApplicationState,
        user: impl Into<Snowflake<User>>,
    ) -> Result<(), RESTError> {
        app.ops().update_fcm_token(user, self).await
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveFCMToken {
    pub token: String,
}

impl RemoveFCMToken {
    /// Perform the remove operation
    ///
    /// This is a shorthand for `app.ops().remove_fcm_token(user, &payload.token).await`
    ///
    /// # Parameters
    ///
    /// - `app` - The application state
    /// - `user` - The user to remove the token from
    ///
    /// # Errors
    ///
    /// - [`sqlx::Error`] if the delete operation fails
    #[inline]
    pub async fn perform_request(
        self,
        app: &ApplicationState,
        user: impl Into<Snowflake<User>>,
    ) -> Result<(), sqlx::Error> {
        app.ops().remove_fcm_token(user, &self.token).await
    }
}
