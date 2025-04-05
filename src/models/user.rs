use std::{hash::Hash, sync::LazyLock};

use chrono::DateTime;
use chrono::prelude::*;
use derive_builder::Builder;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::app::Config;
use crate::gateway::Gateway;

use super::{
    avatar::{Avatar, FullAvatar, PartialAvatar, UserAvatar},
    errors::BuildError,
    omittableoption::OmittableOption,
    request_payloads::{CreateUser, UpdateUser},
    snowflake::Snowflake,
};

static USERNAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([a-z0-9]|[a-z0-9]+(?:[._][a-z0-9]+)*)$").expect("Failed to compile username regex")
});

/// Represents the presence of a user.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(i16)]
pub enum Presence {
    /// The user is currently active.
    Online = 0,
    /// The user is idle or away from the keyboard.
    Away = 1,
    /// The user is busy. Clients should try to disable notifications in this state.
    Busy = 2,
    /// The user is offline or invisible.
    Offline = 3,
}

impl From<i16> for Presence {
    fn from(presence: i16) -> Self {
        match presence {
            0 => Self::Online,
            1 => Self::Away,
            2 => Self::Busy,
            _ => Self::Offline,
        }
    }
}

impl Default for Presence {
    fn default() -> Self {
        Self::Online
    }
}

/// Represents a user record stored in the database.
pub struct UserRecord {
    pub id: Snowflake<User>,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_hash: Option<String>,
    pub last_presence: i16,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Builder)]
#[builder(setter(into), build_fn(error = "BuildError"))]
pub struct User {
    /// The snowflake belonging to this user.
    id: Snowflake<User>,
    /// A user's username. This is unique to the user.
    username: String,
    /// A user's displayname.
    #[builder(default)]
    display_name: Option<String>,

    /// The user's avatar hash.
    #[serde(rename = "avatar_hash")]
    #[builder(default)]
    avatar: Option<Avatar<UserAvatar>>,

    /// The last presence used by this user.
    /// This does not represent the user's actual presence, as that also depends on the gateway connection.
    #[serde(skip)]
    #[builder(default)]
    last_presence: Presence,
    /// Is 'null' in all cases except when the user is sent in a `GUILD_CREATE` event.
    /// This is the presence that is sent in payloads to clients.
    #[serde(rename = "presence")]
    #[builder(setter(skip), default)]
    displayed_presence: Option<Presence>,
}

impl User {
    /// Create a new builder to construct a user.
    pub fn builder() -> UserBuilder {
        UserBuilder::default()
    }

    /// The snowflake belonging to this user.
    pub const fn id(&self) -> Snowflake<Self> {
        self.id
    }

    /// The user's creation date.
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.id.created_at()
    }

    /// The user's username. This is unique to the user.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The user's display name. This is the same as the username unless the user has changed it.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// The user's display name. This is the same as the username unless the user has changed it.
    pub const fn display_name_mut(&mut self) -> Option<&mut String> {
        self.display_name.as_mut()
    }

    /// The user's avatar.
    pub const fn avatar(&self) -> Option<&Avatar<UserAvatar>> {
        self.avatar.as_ref()
    }

    /// The last known presence of the user.
    ///
    /// This does not represent the user's actual presence, as that also depends on the gateway connection.
    pub const fn last_presence(&self) -> &Presence {
        &self.last_presence
    }

    /// Retrieve the user's presence.
    pub async fn presence(&self, gateway: &Gateway) -> &Presence {
        if gateway.is_connected(self.id()).await {
            &self.last_presence
        } else {
            &Presence::Offline
        }
    }

    /// Create a new user from a payload.
    ///
    /// ## Parameters
    ///
    /// * `config` - The application configuration.
    /// * `payload` - The payload containing the user's data.
    ///
    /// ## Errors
    ///
    /// * [`BuildError::ValidationError`] - If the username is invalid.
    ///
    /// ## Returns
    ///
    /// The new user object.
    pub fn from_payload(config: &Config, payload: &CreateUser) -> Result<Self, BuildError> {
        Ok(Self {
            id: Snowflake::gen_new(config),
            username: Self::validate_username(&payload.username)?.to_string(),
            display_name: None,
            avatar: None,
            last_presence: Presence::Online,
            displayed_presence: None,
        })
    }

    /// Build a user object directly from a database record.
    pub fn from_record(record: UserRecord) -> Self {
        Self {
            id: record.id,
            username: record.username,
            avatar: record.avatar_hash.map(|h| {
                Avatar::Partial(
                    PartialAvatar::<UserAvatar>::new(h, record.id).expect("Database should have valid avatar hash"),
                )
            }),
            display_name: record.display_name,
            last_presence: Presence::from(record.last_presence),
            displayed_presence: None,
        }
    }

    /// Update the model with new data.
    ///
    /// ## Arguments
    ///
    /// * `request` - The update request.
    ///
    /// ## Returns
    ///
    /// Whether the user's avatar was updated, requiring an upload to S3.
    ///
    /// ## Errors
    ///
    /// * [`BuildError::ValidationError`] - If the new username is invalid.
    /// * [`BuildError::ValidationError`] - If the new avatar data is invalid.
    ///
    /// ## Note
    ///
    /// The avatar data still needs to be uploaded to S3.
    pub fn update(&mut self, request: UpdateUser) -> Result<bool, BuildError> {
        if let Option::Some(username) = request.username {
            self.set_username(username)?;
        }

        if let OmittableOption::Some(ref display_name) = request.display_name {
            if display_name.len() < 3 {
                return Err(BuildError::ValidationError(
                    "Display name must be at least 3 characters long".to_string(),
                ));
            } else if display_name.len() > 32 {
                return Err(BuildError::ValidationError(
                    "Display name must be at most 32 characters long".to_string(),
                ));
            }
        }

        if let Ok(display_name) = request.display_name.try_into() {
            self.display_name = display_name;
        }

        if let Ok(avatar) = request
            .avatar
            .map(|uri| FullAvatar::from_data_uri(&*self, uri))
            .transpose()?
            .map(Avatar::Full)
            .try_into()
        {
            self.avatar = avatar;
            return Ok(true);
        }

        Ok(false)
    }

    /// Transform this object to also include the user's presence.
    #[must_use]
    pub async fn include_presence(self, gateway: &Gateway) -> Self {
        let presence = self.presence(gateway).await;
        Self {
            displayed_presence: Some(*presence),
            ..self
        }
    }

    /// Validates and sets a new username for this user.
    ///
    /// The username must be committed to the database for the change to take effect.
    ///
    /// ## Errors
    ///
    /// * [`BuilderError::ValidationError`] - If the username is invalid.
    pub fn set_username(&mut self, username: String) -> Result<(), BuildError> {
        Self::validate_username(&username)?;
        self.username = username;
        Ok(())
    }

    /// Validates a username.
    ///
    /// ## Errors
    ///
    /// * [`BuildError::ValidationError`] - If the username is invalid.
    ///
    /// ## Returns
    ///
    /// The username if it is valid.
    fn validate_username(username: &str) -> Result<&str, BuildError> {
        if !USERNAME_REGEX.is_match(username) {
            return Err(BuildError::ValidationError(format!(
                "Invalid username, must match regex: {}",
                USERNAME_REGEX.as_str()
            )));
        }
        if username.len() > 32 || username.len() < 3 {
            return Err(BuildError::ValidationError(
                "Invalid username, must be between 3 and 32 characters long".to_string(),
            ));
        }
        Ok(username)
    }
}

impl From<User> for Snowflake<User> {
    fn from(user: User) -> Self {
        user.id()
    }
}

impl From<&User> for Snowflake<User> {
    fn from(user: &User) -> Self {
        user.id()
    }
}

impl From<&mut User> for Snowflake<User> {
    fn from(user: &mut User) -> Self {
        user.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a dummy User record.
    fn dummy_user() -> User {
        let record = UserRecord {
            id: Snowflake::new(0),
            username: "initial".to_string(),
            display_name: None,
            avatar_hash: None,
            last_presence: 0,
        };
        User::from_record(record)
    }

    #[test]
    fn test_validate_username_valid() {
        // Valid usernames: lowercase letters, digits and dots.
        assert!(User::validate_username("abc").is_ok());
        assert!(User::validate_username("test.user").is_ok());
    }

    #[test]
    fn test_validate_username_invalid() {
        // Too short.
        assert!(User::validate_username("ab").is_err());
        // Too long.
        let long_username = "a".repeat(33);
        assert!(User::validate_username(&long_username).is_err());
        // Contains uppercase letters.
        assert!(User::validate_username("Invalid").is_err());
        // Contains dash which is not allowed by the regex.
        assert!(User::validate_username("test-user").is_err());
        // Contains more than one underscore in a row.
        assert!(User::validate_username("test__user").is_err());
        // Contains non-alphanmeric characters.
        assert!(User::validate_username("test!user").is_err());
    }

    #[test]
    fn test_set_username_valid() {
        let mut user = dummy_user();
        let res = user.set_username("new.valid".to_string());
        assert!(res.is_ok());
        assert_eq!(user.username(), "new.valid");
    }

    #[test]
    fn test_set_username_invalid() {
        let mut user = dummy_user();
        let res = user.set_username("NoUpper".to_string());
        assert!(res.is_err());
    }

    #[test]
    fn test_from_record() {
        let record = UserRecord {
            id: Snowflake::new(42),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            avatar_hash: Some("abc123_png".to_string()),
            last_presence: 1,
        };

        let user = User::from_record(record);

        assert_eq!(user.id(), Snowflake::new(42));
        assert_eq!(user.username(), "testuser");
        assert_eq!(user.display_name(), Some("Test User"));
        assert!(user.avatar().is_some());
        assert_eq!(*user.last_presence(), Presence::Away);
    }

    #[test]
    fn test_update_display_name() {
        let mut user = dummy_user();
        let mut request = UpdateUser {
            username: None,
            display_name: OmittableOption::Omitted,
            avatar: OmittableOption::Omitted,
        };

        // Test valid display name
        request.display_name = OmittableOption::Some("Valid Name".to_string());
        let res = user.update(request.clone());
        assert!(res.is_ok());
        assert_eq!(user.display_name(), Some("Valid Name"));

        // Test too short display name
        request.display_name = OmittableOption::Some("ab".to_string());
        let res = user.update(request.clone());
        assert!(res.is_err());

        // Test too long display name
        let long_name = "a".repeat(33);
        request.display_name = OmittableOption::Some(long_name);
        let res = user.update(request);
        assert!(res.is_err());
    }

    #[test]
    fn test_presence_conversion() {
        assert_eq!(Presence::from(0), Presence::Online);
        assert_eq!(Presence::from(1), Presence::Away);
        assert_eq!(Presence::from(2), Presence::Busy);
        assert_eq!(Presence::from(3), Presence::Offline);
        assert_eq!(Presence::from(99), Presence::Offline); // Any other value defaults to Offline
    }
}
