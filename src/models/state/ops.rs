use chrono::Utc;

use crate::{
    gateway::handler::{ConnectionId, SendMode},
    models::{
        attachment::{Attachment, AttachmentLike, FullAttachment},
        avatar::{Avatar, AvatarLike},
        channel::{Channel, ChannelLike, ChannelRecord, TextChannel},
        errors::{AppError, BuildError, GatewayError, RESTError},
        gateway_event::{GatewayEvent, GatewayMessage},
        guild::{Guild, GuildRecord},
        member::{ExtendedMemberRecord, Member, MemberRecord, UserLike},
        message::{ExtendedMessageRecord, Message},
        requests::{CreateGuild, CreateUser, UpdateGuild, UpdateMessage, UpdateUser},
        snowflake::Snowflake,
        user::{Presence, User, UserRecord},
    },
};

use super::ApplicationState;

/// Contains all the application state operations.
pub struct Ops<'a> {
    app: &'a ApplicationState,
}

impl<'a> Ops<'a> {
    /// Create a new application state operations.
    pub const fn new(app: &'a ApplicationState) -> Self {
        Self { app }
    }

    /// Called when a message is received from a gateway connection.
    ///
    /// ## Arguments
    ///
    /// * `connection_id` - The ID of the connection that received the message.
    /// * `message` - The message that was received.
    pub async fn handle_inbound_gateway_message(&self, connection_id: ConnectionId, message: GatewayMessage) {
        let res = match message {
            GatewayMessage::StartTyping { channel_id } => self.trigger_typing(channel_id, connection_id.0).await,
            GatewayMessage::Identify { .. } => Err(GatewayError::AuthError("Already identified".into())),
            GatewayMessage::Heartbeat => Ok(()),
        };

        if let Err(e) = res {
            self.app
                .gateway()
                .close_session(connection_id, e.close_code(), e.to_string());
        }
    }

    /// Triggers a typing start event for a given user in a channel.
    ///
    /// ## Arguments
    ///
    /// * `channel` - The channel to trigger the typing event in.
    /// * `user` - The user to trigger the typing event for.
    ///
    /// ## Errors
    ///
    /// * [`AppError`] - If the database query fails.
    pub async fn trigger_typing(
        &self,
        channel: impl Into<Snowflake<Channel>>,
        user: impl Into<Snowflake<User>>,
    ) -> Result<(), GatewayError> {
        let channel_id = channel.into();
        let user_id = user.into();

        let record = sqlx::query!(
            r#"SELECT c.guild_id as channel_guild_id, m.guild_id as "member_guild_id?"
            FROM channels c
            LEFT JOIN members m ON m.guild_id = c.guild_id AND m.user_id = $2
            WHERE c.id = $1"#,
            channel_id as Snowflake<Channel>,
            user_id as Snowflake<User>,
        )
        .fetch_optional(self.app.db())
        .await?;

        let record = record.ok_or_else(|| AppError::NotFound("Channel not found".into()))?;
        if record.member_guild_id.is_none() {
            return Err(GatewayError::Forbidden("Cannot access resource".into()));
        }

        let channel_guild_id: Snowflake<Guild> = record.channel_guild_id.into();

        self.app.gateway().dispatch(
            GatewayEvent::TypingStart { user_id, channel_id },
            SendMode::ToGuild(channel_guild_id),
        );

        Ok(())
    }

    /// Fetch a channel from the database by ID.
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the channel to fetch.
    ///
    /// ## Returns
    ///
    /// The channel if found, otherwise `None`.
    pub async fn fetch_channel(&self, id: impl Into<Snowflake<Channel>>) -> Option<Channel> {
        let record = sqlx::query_as!(
            ChannelRecord,
            "SELECT * FROM channels WHERE id = $1",
            id.into() as Snowflake<Channel>
        )
        .fetch_optional(self.app.db())
        .await
        .ok()??;

        Some(Channel::from_record(record))
    }

    /// Create a new channel in the database.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn create_channel(&self, channel: &Channel) -> Result<Channel, sqlx::Error> {
        sqlx::query_as!(
            ChannelRecord,
            "INSERT INTO channels (id, guild_id, name, channel_type)
            VALUES ($1, $2, $3, $4) RETURNING *",
            channel.id() as Snowflake<Channel>,
            channel.guild_id() as Snowflake<Guild>,
            channel.name(),
            channel.channel_type(),
        )
        .fetch_one(self.app.db())
        .await
        .map(Channel::from_record)
    }

    /// Commit this channel to the database.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn update_channel(&self, channel: &Channel) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE channels SET name = $2 WHERE id = $1",
            channel.id() as Snowflake<Channel>,
            channel.name()
        )
        .execute(self.app.db())
        .await?;

        Ok(())
    }

    /// Deletes the channel.
    ///
    /// ## Locks
    ///
    /// * `app().db` (read)
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request to delete all attachments fails.
    /// * [`AppError::Database`] - If the database query fails.
    pub async fn delete_channel(&self, channel: impl Into<Snowflake<Channel>>) -> Result<(), AppError> {
        let channel_id: Snowflake<Channel> = channel.into();

        self.app.s3().remove_all_for_channel(channel_id).await?;

        sqlx::query!("DELETE FROM channels WHERE id = $1", channel_id as Snowflake<Channel>)
            .execute(self.app.db())
            .await?;

        Ok(())
    }

    /// Fetch messages from this channel.
    ///
    /// ## Arguments
    ///
    /// * `limit` - The maximum number of messages to fetch. Defaults to 50, capped at 100.
    /// * `before` - Fetch messages before this ID.
    /// * `after` - Fetch messages after this ID.
    /// * `around` - Fetch messages around this ID. The message will be included if it still exists.
    ///
    /// ## Returns
    ///
    /// [`Vec<Message>`] - The messages fetched. The ordering of the returned messages is unspecified.
    ///
    /// ## Errors
    ///
    /// * [`RESTError::BadRequest`] - If both `before` and `after` are provided.
    /// * [`AppError::Database`] - If the database query fails.
    pub async fn fetch_messages_from(
        &self,
        channel: impl Into<Snowflake<Channel>>,
        limit: Option<u32>,
        before: Option<impl Into<Snowflake<Message>>>,
        after: Option<impl Into<Snowflake<Message>>>,
        around: Option<impl Into<Snowflake<Message>>>,
    ) -> Result<Vec<Message>, RESTError> {
        // Check if more than one of the before/after/around parameters are provided
        if [before.is_some(), after.is_some(), around.is_some()]
            .into_iter()
            .filter(|&b| b)
            .count()
            > 1
        {
            return Err(RESTError::BadRequest(
                "Parameters 'before', 'after', and 'around' are mutually exclusive.".into(),
            ));
        }

        /*
        Note: The messages are first queried in the inner subquery to ensure
        the limits are related to the count of fetched messages and not the final join result's row count.

        Note 2: The ordering depends on if after or before are provided.
        If after is provided, we order by ASC (id) to get the messages right after the `after` message.
        Otherwise, we order by DESC (-id) to get the messages right before the `before` message.
        (Or the latest messages if no before is provided)
        */
        // SAFETY: sqlx doesn't understand LEFT JOIN properly, so we have to use unchecked here.
        let records = if around.is_none() {
            sqlx::query_as_unchecked!(
                ExtendedMessageRecord,
                "SELECT m.*, users.username, users.display_name, users.avatar_hash, 
                        attachments.id AS attachment_id, attachments.filename AS attachment_filename, attachments.content_type AS attachment_content_type
                 FROM (
                     SELECT *
                     FROM messages
                     WHERE channel_id = $1
                       AND ($2::BIGINT IS NULL OR id < $2)
                       AND ($3::BIGINT IS NULL OR id > $3)
                     ORDER BY CASE WHEN $3 IS NOT NULL THEN id ELSE -id END
                     LIMIT $4
                 ) m
                 LEFT JOIN users ON m.user_id = users.id
                 LEFT JOIN attachments ON m.id = attachments.message_id",
                channel.into(),
                before.map(Into::into),
                after.map(Into::into),
                i64::from(limit.unwrap_or(50).min(100))
            )
            .fetch_all(self.app.db())
            .await?
        } else {
            // Ensure the final message count is always the limit (+1 for the anchor message)
            let limit_val = i64::from(limit.unwrap_or(50).clamp(2, 100));
            let before_limit = limit_val / 2;
            let after_limit = limit_val - before_limit;

            sqlx::query_as_unchecked!(
                ExtendedMessageRecord,
                r#"
                SELECT m.*, u.username, u.display_name, u.avatar_hash, 
                       a.id AS attachment_id, a.filename AS attachment_filename, a.content_type AS attachment_content_type
                FROM (
                    (SELECT *
                    FROM messages
                    WHERE channel_id = $1 AND id < $2
                    ORDER BY id DESC
                    LIMIT $3)
                UNION ALL
                    (SELECT *
                    FROM messages
                    WHERE channel_id = $1 AND id >= $2
                    ORDER BY id ASC
                    LIMIT $4)
                ) m
                LEFT JOIN users u ON m.user_id = u.id
                LEFT JOIN attachments a ON m.id = a.message_id
                "#,
                channel.into(),
                around.expect("'around' should exist").into(),
                before_limit,
                after_limit
            )
            .fetch_all(self.app.db())
            .await?
        };

        Ok(Message::from_records(&records)?)
    }

    /// Fetches a guild from the database by ID.
    ///
    /// ## Arguments
    ///
    /// * `guild` - The ID of the guild to fetch.
    pub async fn fetch_guild(&self, guild: impl Into<Snowflake<Guild>>) -> Option<Guild> {
        let record = sqlx::query_as!(
            GuildRecord,
            "SELECT id, name, owner_id, avatar_hash FROM guilds WHERE id = $1",
            guild.into() as Snowflake<Guild>,
        )
        .fetch_optional(self.app.db())
        .await
        .ok()??;

        Some(Guild::from_record(record))
    }

    /// Fetch the owner of the guild.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Build`] - If the member could not be built.
    /// * [`AppError::Database`] - If the database query fails.
    pub async fn fetch_guild_owner(&self, guild: &Guild) -> Result<Member, AppError> {
        self.fetch_member(guild.owner_id(), guild)
            .await
            .transpose()
            .expect("Owner doesn't exist for guild, this should be impossible")
    }

    /// Fetch all members that are in the guild.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn fetch_members_for(&self, guild: impl Into<Snowflake<Guild>>) -> Result<Vec<Member>, AppError> {
        let records = sqlx::query_as!(
            ExtendedMemberRecord,
            "SELECT members.*, users.username, users.display_name, users.avatar_hash, users.last_presence 
            FROM members
            INNER JOIN users ON users.id = members.user_id
            WHERE members.guild_id = $1",
            guild.into() as Snowflake<Guild>
        )
        .fetch_all(self.app.db())
        .await?;

        records
            .into_iter()
            .map(Member::from_extended_record)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    /// Fetch all channels that are in the guild.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn fetch_channels_for(&self, guild: impl Into<Snowflake<Guild>>) -> Result<Vec<Channel>, sqlx::Error> {
        let records = sqlx::query_as!(
            ChannelRecord,
            "SELECT * FROM channels WHERE guild_id = $1",
            guild.into() as Snowflake<Guild>
        )
        .fetch_all(self.app.db())
        .await?;

        Ok(records.into_iter().map(Channel::from_record).collect())
    }

    /// Adds a member to the guild. If the member already exists, does nothing.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn create_member(
        &self,
        guild: impl Into<Snowflake<Guild>>,
        user: impl Into<Snowflake<User>>,
    ) -> Result<Member, sqlx::Error> {
        let user_id = user.into();

        let user = self.fetch_user(user_id).await.ok_or(sqlx::Error::RowNotFound)?;

        let record = sqlx::query_as!(
            MemberRecord,
            "INSERT INTO members (user_id, guild_id, joined_at)
            VALUES ($1, $2, $3) RETURNING *",
            user_id as Snowflake<User>,
            guild.into() as Snowflake<Guild>,
            Utc::now().timestamp(),
        )
        .fetch_one(self.app.db())
        .await?;
        Ok(Member::from_record(user, record))
    }

    /// Removes a member from a guild.
    ///
    /// ## Errors
    ///
    /// * [`RESTError::App`] - If the database query fails.
    /// * [`RESTError::Forbidden`] - If the member is the owner of the guild.
    ///
    /// Note: If the member is the owner of the guild, this will fail.
    pub async fn delete_member(&self, guild: &Guild, user: impl Into<Snowflake<User>>) -> Result<(), RESTError> {
        let user_id = user.into();
        if guild.owner_id() == user_id {
            return Err(RESTError::Forbidden("Cannot remove owner from guild".into()));
        }

        sqlx::query!(
            "DELETE FROM members WHERE user_id = $1 AND guild_id = $2",
            user_id as Snowflake<User>,
            guild.id() as Snowflake<Guild>,
        )
        .execute(self.app.db())
        .await?;
        Ok(())
    }

    /// Fetch a member from the database by id and guild id.
    ///
    /// ## Arguments
    ///
    /// * `user` - The ID of the user to fetch.
    /// * `guild` - The ID of the guild the user is in.
    ///
    /// ## Returns
    ///
    /// The member if found, otherwise `None`.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Database`] - If the database query fails.
    /// * [`AppError::Build`] - If the member could not be built.
    pub async fn fetch_member(
        &self,
        user: impl Into<Snowflake<User>>,
        guild: impl Into<Snowflake<Guild>>,
    ) -> Result<Option<Member>, AppError> {
        let record = sqlx::query_as!(
            ExtendedMemberRecord,
            "SELECT members.*, users.username, users.display_name, users.avatar_hash, users.last_presence 
            FROM members
            INNER JOIN users ON users.id = members.user_id
            WHERE members.user_id = $1 AND members.guild_id = $2",
            user.into() as Snowflake<User>,
            guild.into() as Snowflake<Guild>,
        )
        .fetch_optional(self.app.db())
        .await?;

        record.map(Member::from_extended_record).transpose().map_err(Into::into)
    }

    /// Commit the member to the database.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn update_member(&self, member: &Member) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO members (user_id, guild_id, nickname, joined_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id, guild_id) DO UPDATE
            SET nickname = $3, joined_at = $4",
            member.user().id() as Snowflake<User>,
            member.guild_id() as Snowflake<Guild>,
            member.nickname().as_ref(),
            member.joined_at()
        )
        .execute(self.app.db())
        .await?;

        //self.app.ops().update_user(member.user()).await?;

        Ok(())
    }

    /// Create a new guild
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    ///
    /// ## Returns
    ///
    /// * [`Guild`] - The created guild.
    /// * [`Channel`] - The general text channel for the guild.
    /// * [`Member`] - The owner of the guild.
    ///
    /// Note: This will also create a general text channel for the guild.
    pub async fn create_guild(
        &self,
        payload: CreateGuild,
        owner: impl Into<Snowflake<User>>,
    ) -> Result<(Guild, Channel, Member), sqlx::Error> {
        let guild = Guild::from_payload(&self.app.config, payload, owner);
        sqlx::query!(
            "INSERT INTO guilds (id, name, owner_id)
            VALUES ($1, $2, $3)",
            guild.id() as Snowflake<Guild>,
            guild.name(),
            guild.owner_id() as Snowflake<User>,
        )
        .execute(self.app.db())
        .await?;

        let member = self.create_member(&guild, guild.owner_id()).await?;

        let general = TextChannel::new(guild.id().cast(), &guild, "general".to_string()).into();
        self.app.ops().create_channel(&general).await?;
        Ok((guild, general, member))
    }

    /// Commits the current state of this guild object to the database.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn update_guild(&self, payload: UpdateGuild, old_guild: &Guild) -> Result<Guild, AppError> {
        let mut guild = old_guild.clone();
        let needs_s3_update = guild.update(payload)?;

        if old_guild == &guild {
            return Ok(guild);
        }

        if needs_s3_update {
            match guild.avatar() {
                Some(Avatar::Full(f)) => f.upload(self.app.s3()).await?,
                Some(Avatar::Partial(_)) => {
                    Err(BuildError::IllegalState("Cannot upload partial avatar".into()))?;
                }
                None => {}
            }

            if let Some(a) = old_guild.avatar() {
                a.delete(self.app.s3()).await?;
            }
        }

        let record = sqlx::query_as!(
            GuildRecord,
            "UPDATE guilds
            SET name = $2, owner_id = $3, avatar_hash = $4
            WHERE id = $1 RETURNING *",
            guild.id() as Snowflake<Guild>,
            guild.name(),
            guild.owner_id() as Snowflake<User>,
            guild.avatar().map(AvatarLike::avatar_hash),
        )
        .fetch_one(self.app.db())
        .await?;
        Ok(Guild::from_record(record))
    }

    /// Deletes the guild.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request to delete all attachments fails.
    /// * [`AppError::Database`] - If the database query fails.
    pub async fn delete_guild(&self, guild: impl Into<Snowflake<Guild>>) -> Result<(), AppError> {
        let guild_id: Snowflake<Guild> = guild.into();

        self.app.s3().remove_all_for_guild(guild_id).await?;

        sqlx::query!("DELETE FROM guilds WHERE id = $1", guild_id as Snowflake<Guild>)
            .execute(self.app.db())
            .await?;
        Ok(())
    }

    /// Retrieve a message and fetch its author from the database in one query.
    /// Attachment contents will not be retrieved from S3.
    ///
    /// ## Arguments
    ///
    /// * `message` - The ID of the message to retrieve.
    ///
    /// ## Returns
    ///
    /// The message if found, otherwise `None`.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Database`] - If the database query fails.
    /// * [`AppError::Build`] - If the message is malformed.
    pub async fn fetch_message(&self, message: impl Into<Snowflake<Message>>) -> Result<Option<Message>, AppError> {
        // sqlx cannot handle LEFT JOIN properly, so we have to use unchecked here.
        let records = sqlx::query_as_unchecked!(
            ExtendedMessageRecord,
            "SELECT messages.*, users.username, users.display_name, users.avatar_hash, attachments.id AS attachment_id, attachments.filename AS attachment_filename, attachments.content_type AS attachment_content_type
            FROM messages
            LEFT JOIN users ON messages.user_id = users.id
            LEFT JOIN attachments ON messages.id = attachments.message_id
            WHERE messages.id = $1",
            message.into() as Snowflake<Message>
        )
        .fetch_all(self.app.db())
        .await?;

        Ok(Message::from_records(&records)?.pop())
    }

    /// Commit this message to the database. Uploads all attachments to S3.
    /// It is highly recommended to call [`Message::strip_attachment_contents`] after calling
    /// this method to remove the attachment contents from memory.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request to upload one of the attachments fails.
    /// * [`AppError::Database`] - If the database request fails.
    pub async fn commit_message(&self, message: &Message) -> Result<(), AppError> {
        sqlx::query!(
            "INSERT INTO messages (id, user_id, channel_id, content, edited)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE
            SET user_id = $2, channel_id = $3, content = $4, edited = $5",
            message.id() as Snowflake<Message>,
            message.author().map(UserLike::id) as Option<Snowflake<User>>,
            message.channel_id() as Snowflake<Channel>,
            message.content(),
            message.edited(),
        )
        .execute(self.app.db())
        .await?;

        for attachment in message.attachments() {
            if let Attachment::Full(f) = attachment {
                self.create_attachment(f).await?;
            }
        }
        Ok(())
    }

    /// Update a message in the database based on an update payload.
    ///
    /// ## Arguments
    ///
    /// * `message` - The message to update.
    /// * `payload` - The update payload.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Database`] - If the database query fails.
    /// * [`AppError::NotFound`] - If the message does not exist.
    ///
    /// ## Returns
    ///
    /// The updated message if the commit was successful.
    pub async fn update_message(
        &self,
        message: impl Into<Snowflake<Message>>,
        payload: UpdateMessage,
    ) -> Result<Message, AppError> {
        let message_id = message.into();

        let mut message = self
            .fetch_message(message_id)
            .await?
            .ok_or(AppError::NotFound("Message not found".into()))?;

        message.apply_update(payload);

        self.commit_message(&message).await?;

        Ok(message)
    }

    /// Delete a message.
    ///
    /// ## Arguments
    ///
    /// * `message` - The message to delete.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Database`] - If the database query fails.
    pub async fn delete_message(
        &self,
        channel: impl Into<Snowflake<Channel>>,
        message: impl Into<Snowflake<Message>>,
    ) -> Result<(), AppError> {
        let message_id = message.into();

        sqlx::query!("DELETE FROM messages WHERE id = $1", message_id as Snowflake<Message>)
            .execute(self.app.db())
            .await?;

        self.app.s3().remove_all_for_message(channel, message_id).await?;

        Ok(())
    }

    /// Retrieve a user from the database by their ID.
    ///
    /// ## Arguments
    ///
    /// * `user` - The ID of the user to retrieve.
    ///
    /// ## Returns
    ///
    /// The user if found, otherwise `None`.
    pub async fn fetch_user(&self, user: impl Into<Snowflake<User>>) -> Option<User> {
        let row = sqlx::query_as!(
            UserRecord,
            "SELECT id, username, display_name, avatar_hash, last_presence
            FROM users
            WHERE id = $1",
            user.into() as Snowflake<User>
        )
        .fetch_optional(self.app.db())
        .await
        .ok()??;

        Some(User::from_record(row))
    }

    /// Fetch the presence of a user.
    ///
    /// ## Arguments
    ///
    /// * `user` - The ID of the user to retrieve the presence of.
    ///
    /// ## Returns
    ///
    /// The presence of the user if found, otherwise `None`.
    pub async fn fetch_presence(&self, user: impl Into<Snowflake<User>>) -> Option<Presence> {
        let row = sqlx::query!(
            "SELECT last_presence
            FROM users
            WHERE id = $1",
            user.into() as Snowflake<User>
        )
        .fetch_optional(self.app.db())
        .await
        .ok()??;

        Some(Presence::from(row.last_presence))
    }

    /// Retrieve a user from the database by their username.
    ///
    /// ## Arguments
    ///
    /// * `username` - The username of the user to retrieve.
    ///
    /// ## Returns
    ///
    /// The user if found, otherwise `None`.
    pub async fn fetch_user_by_username(&self, username: &str) -> Option<User> {
        let row = sqlx::query_as!(
            UserRecord,
            "SELECT id, username, display_name, avatar_hash, last_presence
            FROM users
            WHERE username = $1
            LIMIT 1",
            username
        )
        .fetch_optional(self.app.db())
        .await
        .ok()??;

        Some(User::from_record(row))
    }

    /// Fetch all guilds that this user is a member of.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn fetch_guilds_for(&self, user: impl Into<Snowflake<User>>) -> Result<Vec<Guild>, sqlx::Error> {
        let records = sqlx::query_as!(
            GuildRecord,
            "SELECT guilds.id, guilds.name, guilds.owner_id, guilds.avatar_hash
            FROM guilds
            INNER JOIN members ON members.guild_id = guilds.id
            WHERE members.user_id = $1",
            user.into() as Snowflake<User>
        )
        .fetch_all(self.app.db())
        .await?;

        Ok(records.into_iter().map(Guild::from_record).collect())
    }

    /// Fetch all guild IDs that this user is a member of.
    /// This is a more efficient version of [`Ops::fetch_guilds_for`] if you only need the IDs.
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to fetch guild IDs for.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn fetch_guild_ids_for(
        &self,
        user: impl Into<Snowflake<User>>,
    ) -> Result<Vec<Snowflake<Guild>>, sqlx::Error> {
        let records = sqlx::query!(
            "SELECT guild_id
            FROM members
            WHERE user_id = $1",
            user.into() as Snowflake<User>
        )
        .fetch_all(self.app.db())
        .await?;

        Ok(records.into_iter().map(|r| r.guild_id.into()).collect())
    }

    /// Create a new user in the database.
    ///
    /// ## Errors
    ///
    /// * [`sqlx::Error`] - If the database query fails.
    pub async fn create_user(&self, payload: CreateUser) -> Result<User, AppError> {
        let user = User::from_payload(&self.app.config, &payload)?;

        sqlx::query!(
            "INSERT INTO users (id, username)
            VALUES ($1, $2)",
            user.id() as Snowflake<User>,
            payload.username,
        )
        .execute(self.app.db())
        .await?;

        Ok(user)
    }

    /// Apply an update payload to the user.
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to update.
    /// * `payload` - The update payload.
    ///
    /// ## Errors
    ///
    /// * [`AppError::Database`] - If the database query fails.
    /// * [`AppError::NotFound`] - If the user does not exist.
    /// * [`AppError::Build`] - If the avatar is partial.
    ///
    /// ## Returns
    ///
    /// The user if the commit was successful.
    pub async fn update_user(&self, user: impl Into<Snowflake<User>>, payload: UpdateUser) -> Result<User, AppError> {
        let user_id = user.into();

        let old_user = self
            .fetch_user(user_id)
            .await
            .ok_or(AppError::NotFound("User not found".into()))?;

        let mut user = old_user.clone();
        let needs_s3_update = user.update(payload)?;

        if old_user == user {
            return Ok(user);
        }

        if needs_s3_update {
            match user.avatar() {
                Some(Avatar::Full(f)) => f.upload(self.app.s3()).await?,
                Some(Avatar::Partial(_)) => {
                    return Err(BuildError::IllegalState("Cannot upload partial avatar".into()).into());
                }
                None => {}
            }

            if let Some(a) = old_user.avatar() {
                a.delete(self.app.s3()).await?;
            }
        }

        let record = sqlx::query_as!(
            UserRecord,
            "UPDATE users SET username = $2, display_name = $3, last_presence = $4, avatar_hash = $5
            WHERE id = $1 RETURNING *",
            user_id as Snowflake<User>,
            user.username(),
            user.display_name(),
            *user.last_presence() as i16,
            user.avatar().map(AvatarLike::avatar_hash),
        )
        .fetch_one(self.app.db())
        .await?;
        Ok(User::from_record(record))
    }

    /// Commit the attachment to the database. Uploads the contents to S3 implicitly.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn create_attachment(&self, attachment: &FullAttachment) -> Result<(), AppError> {
        attachment.upload(self.app.s3()).await?;

        sqlx::query!(
            "INSERT INTO attachments (id, filename, message_id, channel_id, content_type)
            VALUES ($1, $2, $3, $4, $5) 
            ON CONFLICT (id, message_id) 
            DO UPDATE SET filename = $2, content_type = $5",
            i32::from(attachment.id()),
            attachment.filename(),
            attachment.message_id() as Snowflake<Message>,
            attachment.channel_id() as Snowflake<Channel>,
            attachment.mime().to_string(),
        )
        .execute(self.app.db())
        .await?;

        Ok(())
    }
}
