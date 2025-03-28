use serde::Serialize;

use crate::app::Config;

use super::{
    avatar::{Avatar, FullAvatar, GuildAvatar, PartialAvatar},
    errors::AppError,
    request_payloads::{CreateGuild, UpdateGuild},
    snowflake::Snowflake,
    user::User,
};

pub struct GuildRecord {
    pub id: Snowflake<Guild>,
    pub name: String,
    pub owner_id: Snowflake<User>,
    pub avatar_hash: Option<String>,
}

/// Represents a guild.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Guild {
    id: Snowflake<Self>,
    name: String,
    owner_id: Snowflake<User>,

    #[serde(rename = "avatar_hash")]
    avatar: Option<Avatar<GuildAvatar>>,
}

impl Guild {
    /// Create a new guild with the given id, name, and owner id.
    ///
    /// ## Arguments
    ///
    /// * `id` - The guild's ID.
    /// * `name` - The guild's name.
    /// * `owner` - The guild's owner.
    pub fn new(id: Snowflake<Self>, name: String, owner: impl Into<Snowflake<User>>) -> Self {
        Self {
            id,
            name,
            owner_id: owner.into(),
            avatar: None,
        }
    }

    /// The guild's ID.
    pub const fn id(&self) -> Snowflake<Self> {
        self.id
    }

    /// The guild's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The guild's owner's ID.
    pub const fn owner_id(&self) -> Snowflake<User> {
        self.owner_id
    }

    /// The guild's avatar.
    pub const fn avatar(&self) -> Option<&Avatar<GuildAvatar>> {
        self.avatar.as_ref()
    }

    /// Create a new guild object from a database record.
    pub fn from_record(record: GuildRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            owner_id: record.owner_id,
            avatar: record.avatar_hash.map(|h| {
                Avatar::Partial(
                    PartialAvatar::<GuildAvatar>::new(h, record.id).expect("Database should have valid avatar hash"),
                )
            }),
        }
    }

    /// Constructs a new guild from a payload and owner ID.
    ///
    /// ## Arguments
    ///
    /// * `payload` - The payload to construct the guild from.
    /// * `owner` - The ID of the guild's owner.
    pub fn from_payload(config: &Config, payload: CreateGuild, owner: impl Into<Snowflake<User>>) -> Self {
        Self::new(Snowflake::gen_new(config), payload.name, owner.into())
    }

    /// Update the guild with the given payload.
    ///
    /// ## Arguments
    ///
    /// * `payload` - The update payload.
    ///
    /// ## Returns
    ///
    /// Whether the guild's avatar was updated, requiring an upload to S3.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Build`] - If the avatar data URI is invalid.
    pub fn update(&mut self, payload: UpdateGuild) -> Result<bool, AppError> {
        if let Some(name) = payload.name {
            if !(3..=32).contains(&name.len()) {
                return Err(AppError::IllegalArgument(
                    "Guild name must be between 3 and 32 characters".to_string(),
                ));
            }

            self.name = name;
        }
        if let Some(owner_id) = payload.owner_id {
            self.owner_id = owner_id;
        }

        if let Ok(avatar) = payload
            .avatar
            .map(|uri| FullAvatar::from_data_uri(self.id(), uri))
            .transpose()?
            .map(Avatar::Full)
            .try_into()
        {
            let changed = self.avatar != avatar;
            self.avatar = avatar;
            return Ok(changed);
        }

        Ok(false)
    }
}

impl From<Guild> for Snowflake<Guild> {
    fn from(guild: Guild) -> Self {
        guild.id()
    }
}

impl From<&Guild> for Snowflake<Guild> {
    fn from(guild: &Guild) -> Self {
        guild.id()
    }
}

impl From<&mut Guild> for Snowflake<Guild> {
    fn from(guild: &mut Guild) -> Self {
        guild.id()
    }
}

#[cfg(test)]
mod tests {
    use crate::models::omittableoption::OmittableOption;

    use super::*;

    #[test]
    fn test_new() {
        let id = Snowflake::new(1);
        let name = "Test Guild".to_string();
        let owner_id = Snowflake::<User>::new(2);

        let guild = Guild::new(id, name.clone(), owner_id);

        assert_eq!(guild.id(), id);
        assert_eq!(guild.name(), name);
        assert_eq!(guild.owner_id(), owner_id);
        assert_eq!(guild.avatar(), None);
    }

    #[test]
    fn test_from_record() {
        let id = Snowflake::new(1);
        let name = "Test Guild".to_string();
        let owner_id = Snowflake::<User>::new(2);
        let avatar_hash = Some("avatar_hash_png".to_string());

        let record = GuildRecord {
            id,
            name: name.clone(),
            owner_id,
            avatar_hash,
        };

        let guild = Guild::from_record(record);

        assert_eq!(guild.id(), id);
        assert_eq!(guild.name(), name);
        assert_eq!(guild.owner_id(), owner_id);
        assert!(guild.avatar().is_some());
    }

    #[test]
    fn test_update_name() {
        let id = Snowflake::new(1);
        let name = "Test Guild".to_string();
        let owner_id = Snowflake::<User>::new(2);

        let mut guild = Guild::new(id, name, owner_id);

        let new_name = "Updated Guild".to_string();
        let update_payload = UpdateGuild {
            name: Some(new_name.clone()),
            owner_id: None,
            avatar: OmittableOption::None,
        };

        let result = guild.update(update_payload);
        assert!(!result.expect("Should be Ok"));
        assert_eq!(guild.name(), new_name);
    }

    #[test]
    fn test_update_owner() {
        let id = Snowflake::new(1);
        let name = "Test Guild".to_string();
        let owner_id = Snowflake::<User>::new(2);

        let mut guild = Guild::new(id, name, owner_id);

        let new_owner_id = Snowflake::<User>::new(3);
        let update_payload = UpdateGuild {
            name: None,
            owner_id: Some(new_owner_id),
            avatar: OmittableOption::None,
        };

        let result = guild.update(update_payload);
        assert!(!result.expect("Should be Ok"));
        assert_eq!(guild.owner_id(), new_owner_id);
    }

    #[test]
    fn test_update_invalid_name() {
        let id = Snowflake::new(1);
        let name = "Test Guild".to_string();
        let owner_id = Snowflake::<User>::new(2);

        let mut guild = Guild::new(id, name.clone(), owner_id);

        // Test with too short name
        let update_payload = UpdateGuild {
            name: Some("ab".to_string()),
            owner_id: None,
            avatar: OmittableOption::None,
        };

        let result = guild.update(update_payload);
        assert!(result.is_err());
        assert_eq!(guild.name(), name);

        // Test with too long name
        let update_payload = UpdateGuild {
            name: Some("a".repeat(33)),
            owner_id: None,
            avatar: OmittableOption::None,
        };

        let result = guild.update(update_payload);
        assert!(result.is_err());
        assert_eq!(guild.name(), name);
    }

    #[test]
    fn test_snowflake_from_conversions() {
        let id = Snowflake::new(1);
        let guild = Guild::new(id, "Test Guild".to_string(), Snowflake::<User>::new(2));

        let id_from_guild: Snowflake<Guild> = guild.clone().into();
        assert_eq!(id_from_guild, id);

        let id_from_ref: Snowflake<Guild> = (&guild).into();
        assert_eq!(id_from_ref, id);

        let mut guild_mut = guild;
        let id_from_mut_ref: Snowflake<Guild> = (&mut guild_mut).into();
        assert_eq!(id_from_mut_ref, id);
    }
}
