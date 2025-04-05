use axum::extract::Multipart;
use chrono::{DateTime, Utc};
use derive_builder::Builder;
use itertools::Itertools;
use serde::Serialize;

use crate::app::Config;

use super::{
    attachment::{Attachment, AttachmentLike, FullAttachment},
    avatar::{Avatar, PartialAvatar},
    channel::Channel,
    errors::{BuildError, RESTError},
    member::UserLike,
    request_payloads::{CreateMessage, UpdateMessage},
    snowflake::Snowflake,
    user::User,
};

/// Represents a message record stored in the database.
pub struct MessageRecord {
    pub id: Snowflake<Message>,
    pub channel_id: Snowflake<Channel>,
    pub user_id: Option<Snowflake<User>>,
    pub edited: bool,
    pub content: String,
}

/// Represents a message record with associated author data as queried.
/// All associated author fields are optional because the author may have been deleted.
pub struct ExtendedMessageRecord {
    pub id: i64,
    pub channel_id: i64,
    pub content: Option<String>,
    pub user_id: Option<Snowflake<User>>,
    pub edited: bool,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_hash: Option<String>,
    pub attachment_id: Option<i32>,
    pub attachment_filename: Option<String>,
    pub attachment_content_type: Option<String>,
}

/// A chat message.
#[derive(Serialize, Debug, Clone, Builder)]
#[builder(setter(into), build_fn(validate = "Self::validate", error = "BuildError"))]
pub struct Message {
    /// The id of the message.
    id: Snowflake<Message>,

    /// The id of the channel this message was sent in.
    channel_id: Snowflake<Channel>,

    /// The author of the message. This may be none if the author has been deleted since.
    #[builder(setter(strip_option))]
    author: Option<UserLike>,

    /// A nonce that can be used by a client to determine if the message was sent.
    /// The nonce is not stored in the database and thus is not returned by REST calls.
    #[builder(default)]
    nonce: Option<String>,

    /// If true, the message was edited before.
    #[builder(default = "false")]
    edited: bool,

    /// The content of the message.
    #[builder(default)]
    content: Option<String>,

    /// Attachments sent with this message.
    #[builder(default)]
    attachments: Vec<Attachment>,
}

impl MessageBuilder {
    fn validate(&self) -> Result<(), String> {
        if self.content.is_none()
            && (self.attachments.is_none() || self.attachments.as_ref().is_some_and(Vec::is_empty))
        {
            Err("Message must have content or attachments".to_string())
        } else {
            Ok(())
        }
    }
}

impl Message {
    /// Create a new builder for a message.
    pub fn builder() -> MessageBuilder {
        MessageBuilder::default()
    }

    /// The unique ID of this message.
    pub const fn id(&self) -> Snowflake<Self> {
        self.id
    }

    /// The user who sent this message.
    ///
    /// This may be `None` if the author has been deleted since.
    pub const fn author(&self) -> Option<&UserLike> {
        self.author.as_ref()
    }

    /// The ID of the channel this message was sent in.
    pub const fn channel_id(&self) -> Snowflake<Channel> {
        self.channel_id
    }

    /// If true, the message was edited before.
    pub const fn edited(&self) -> bool {
        self.edited
    }

    /// The time at which this message was sent.
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.id.created_at()
    }

    /// A nonce that can be used by a client to determine if the message was sent.
    /// The nonce is not stored in the database and thus is not returned by REST calls.
    pub fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref()
    }

    /// The content of the message.
    pub fn content(&self) -> Option<&str> {
        self.content.as_deref()
    }

    /// Mutable handle to the content of the message.
    pub const fn content_mut(&mut self) -> Option<&mut String> {
        self.content.as_mut()
    }

    /// The attachments sent with this message.
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    /// Create a new message or messages from the given records. Multiple records are linked together by their ID.
    ///
    /// ## Errors
    ///
    /// * [`BuildError`] - If the records are invalid
    pub fn from_records(records: impl IntoIterator<Item = ExtendedMessageRecord>) -> Result<Vec<Self>, BuildError> {
        records
            .into_iter()
            .into_grouping_map_by(|r| Snowflake::<Self>::from(r.id))
            .aggregate(|msg, _id, entry| {
                // First entry, aggregate is None
                match msg {
                    None => {
                        let attachment = (&entry)
                            .try_into()
                            .map(Attachment::Partial)
                            .map_or_else(|_| Vec::new(), |a| vec![a]);

                        let author = {
                            if let Some(user_id) = entry.user_id {
                                let avatar = match entry
                                    .avatar_hash
                                    .map(|h| PartialAvatar::new(h, user_id).map(Avatar::Partial))
                                {
                                    Some(Ok(avatar)) => Some(avatar),
                                    Some(Err(e)) => return Some(Err(e)),
                                    None => None,
                                };

                                let user = match User::builder()
                                    .id(user_id)
                                    .username(entry.username.expect("User should have username")) // This is fine because user_id is not None.
                                    .display_name(entry.display_name)
                                    .avatar(avatar)
                                    .build()
                                {
                                    Ok(user) => user,
                                    Err(e) => return Some(Err(e)),
                                };

                                Some(UserLike::User(user))
                            } else {
                                None
                            }
                        };

                        Some(Ok(Self {
                            id: entry.id.into(),
                            channel_id: entry.channel_id.into(),
                            edited: entry.edited,
                            author,
                            content: entry.content,
                            nonce: None,
                            attachments: attachment,
                        }))
                    }
                    // An aggregate value already exists, append the attachment to the message
                    Some(Ok(mut msg)) => {
                        if let Ok(attachment) = entry.try_into().map(Attachment::Partial) {
                            msg.attachments.push(attachment);
                        }

                        Some(Ok(msg))
                    }
                    // Pass the error through, the build failed upstream
                    Some(Err(e)) => Some(Err(e)),
                }
            })
            .into_values()
            .collect()
    }

    /// Apply an update to the message.
    ///
    /// This will update the message with the provided update payload.
    ///
    /// ## Parameters
    ///
    /// - `payload` - The update message payload
    pub fn apply_update(&mut self, payload: UpdateMessage) {
        if let Ok(mut content) = Option::try_from(payload.content) {
            content = content.map(|c: String| c.trim().to_string());
            self.edited = self.content != content;
            self.content = content;
        }
    }

    /// Create a new message from the given formdata. Assigns a new snowflake to the message.
    ///
    /// ## Errors
    ///
    /// * [`RESTError`] - If the formdata is invalid
    pub async fn from_formdata(
        config: &Config,
        author: UserLike,
        channel: impl Into<Snowflake<Channel>>,
        mut form: Multipart,
    ) -> Result<Self, RESTError> {
        let id = Snowflake::gen_new(config);
        let channel_id: Snowflake<Channel> = channel.into();
        let mut attachments: Vec<Attachment> = Vec::new();
        let mut builder = Self::builder();

        builder.id(id).channel_id(channel_id).author(author);

        while let Some(part) = form.next_field().await? {
            if part.name() == Some("json") && part.content_type().is_some_and(|ct| ct == "application/json") {
                let Ok(data) = part.bytes().await else {
                    return Err(RESTError::MalformedField("json".to_string()));
                };
                let payload = serde_json::from_slice::<CreateMessage>(&data)?;
                builder
                    .content(payload.content.map(|c| c.trim().to_string()))
                    .nonce(payload.nonce.clone());
            } else {
                let attachment = FullAttachment::try_from_field(part, channel_id, id).await?;

                if attachments.iter().any(|a| a.id() == attachment.id()) {
                    return Err(RESTError::DuplicateField("attachment.id".to_string()));
                }
                attachments.push(Attachment::Full(attachment));
            }
        }

        Ok(builder.attachments(attachments).build()?)
    }

    /// Turns all attachments into partial attachments, removing the attachment contents from memory.
    #[must_use]
    pub fn strip_attachment_contents(mut self) -> Self {
        self.attachments = self
            .attachments
            .into_iter()
            .map(|a| {
                if let Attachment::Full(f) = a {
                    Attachment::Partial(f.into())
                } else {
                    a
                }
            })
            .collect();
        self
    }
}

impl From<Message> for Snowflake<Message> {
    fn from(message: Message) -> Self {
        message.id()
    }
}

impl From<&Message> for Snowflake<Message> {
    fn from(message: &Message) -> Self {
        message.id()
    }
}

#[cfg(test)]
mod tests {
    use rand::seq::SliceRandom;

    use crate::models::{attachment::PartialAttachment, avatar::AvatarLike, omittableoption::OmittableOption};

    use super::*;

    fn dummy_message() -> Message {
        Message::builder()
            .id(Snowflake::new(123))
            .channel_id(Snowflake::new(456))
            .content(Some("Test content".to_string()))
            .author(UserLike::User(
                User::builder()
                    .id(Snowflake::new(789))
                    .username("testuser")
                    .avatar(Avatar::Partial(
                        PartialAvatar::new("avatar_hash_png".to_string(), Snowflake::new(789))
                            .expect("Failed to build avatar"),
                    ))
                    .build()
                    .expect("Failed to build user"),
            ))
            .build()
            .expect("Failed to build message")
    }

    #[test]
    fn test_message_builder_validation() {
        // Valid: has content
        let result = Message::builder()
            .id(Snowflake::new(1))
            .channel_id(Snowflake::new(2))
            .content(Some("Hello".to_string()))
            .author(UserLike::User(
                User::builder()
                    .id(Snowflake::new(789))
                    .username("testuser")
                    .build()
                    .expect("Failed to build user"),
            ))
            .build();
        assert!(result.is_ok());

        // Valid: has attachments but no content
        let result = Message::builder()
            .id(Snowflake::new(1))
            .channel_id(Snowflake::new(2))
            .author(UserLike::User(
                User::builder()
                    .id(Snowflake::new(789))
                    .username("testuser")
                    .build()
                    .expect("Failed to build user"),
            ))
            .attachments(vec![Attachment::Partial(PartialAttachment::new(
                0,
                "test.txt".to_string(),
                "text/plain".to_string(),
                Snowflake::new(2),
                Snowflake::new(1),
            ))])
            .build();
        assert!(result.is_ok());

        // Invalid: no content and no attachments
        let result = Message::builder()
            .id(Snowflake::new(1))
            .author(UserLike::User(
                User::builder()
                    .id(Snowflake::new(789))
                    .username("testuser")
                    .build()
                    .expect("Failed to build user"),
            ))
            .channel_id(Snowflake::new(2))
            .build();
        assert!(result.is_err());

        // Invalid: has no author
        let result = Message::builder()
            .id(Snowflake::new(1))
            .channel_id(Snowflake::new(2))
            .content(Some("Hello".to_string()))
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_message_getters() {
        let user = User::builder()
            .id(Snowflake::new(789))
            .username("testuser")
            .build()
            .expect("Failed to build user");

        let attachment = Attachment::Partial(PartialAttachment::new(
            0,
            "test.txt".to_string(),
            "text/plain".to_string(),
            Snowflake::new(2),
            Snowflake::new(1),
        ));

        let message = Message::builder()
            .id(Snowflake::new(123))
            .channel_id(Snowflake::new(456))
            .author(UserLike::User(user))
            .content(Some("Test content".to_string()))
            .nonce(Some("test-nonce".to_string()))
            .edited(true)
            .attachments(vec![attachment])
            .build()
            .expect("Failed to build message");

        assert_eq!(message.id(), Snowflake::new(123));
        assert_eq!(message.channel_id(), Snowflake::new(456));
        assert!(message.author().is_some());
        assert_eq!(message.content(), Some("Test content"));
        assert_eq!(message.nonce(), Some("test-nonce"));
        assert!(message.edited());
        assert!(!message.attachments().is_empty());
    }

    #[test]
    fn test_apply_update() {
        let mut message = dummy_message();
        assert_eq!(message.content(), Some("Test content"));
        assert!(!message.edited());

        // Update content
        let update = UpdateMessage {
            content: OmittableOption::Some("Updated content".to_string()),
        };

        message.apply_update(update);

        assert_eq!(message.content(), Some("Updated content"));
        assert!(message.edited());

        // Update with same content shouldn't change edited flag
        let update = UpdateMessage {
            content: OmittableOption::Some("Updated content".to_string()),
        };

        message.edited = false;
        message.apply_update(update);

        assert_eq!(message.content(), Some("Updated content"));
        assert!(!message.edited());
    }

    #[test]
    fn test_from_records_empty() {
        let records: Vec<ExtendedMessageRecord> = Vec::new();
        let result = Message::from_records(records);
        assert!(result.expect("expected Ok").is_empty());
    }

    #[test]
    fn test_from_records_single() {
        let records = {
            (0..5)
                .map(|i| ExtendedMessageRecord {
                    id: 0,
                    channel_id: 1,
                    content: Some("Test content".to_string()),
                    user_id: Some(Snowflake::new(2)),
                    edited: false,
                    username: Some("testuser".to_string()),
                    display_name: Some("Test User".to_string()),
                    avatar_hash: Some("avatar_hash_png".to_string()),
                    attachment_id: Some(i),
                    attachment_filename: Some("test.txt".to_string()),
                    attachment_content_type: Some("text/plain".to_string()),
                })
                .collect::<Vec<_>>()
        };

        let result = Message::from_records(records);
        let messages = result.expect("expected Ok");
        assert!(messages.len() == 1);

        let message = &messages[0];

        assert_eq!(message.id(), Snowflake::new(0));
        assert_eq!(message.channel_id(), Snowflake::new(1));
        assert_eq!(message.content(), Some("Test content"));
        assert!(message.author().is_some());
        assert_eq!(message.author().expect("Should have author").username(), "testuser");
        assert_eq!(
            message.author().expect("Should have author").display_name(),
            Some("Test User")
        );
        assert_eq!(
            message
                .author()
                .expect("Should have author")
                .avatar()
                .expect("Should have avatar")
                .avatar_hash(),
            "avatar_hash_png"
        );
        assert_eq!(message.attachments().len(), 5);
        assert_eq!(message.attachments()[0].id(), 0);
        assert_eq!(message.attachments()[0].filename(), "test.txt");
        assert_eq!(message.attachments()[0].mime(), mime::TEXT_PLAIN);
        assert_eq!(message.attachments()[0].channel_id(), Snowflake::new(1));
        assert_eq!(message.attachments()[0].message_id(), Snowflake::new(0));
    }

    #[test]
    fn test_from_records_multiple() {
        let mut records = {
            (0..25)
                .map(|i| ExtendedMessageRecord {
                    id: i % 5,
                    channel_id: 1,
                    content: Some("Test content".to_string()),
                    user_id: Some(Snowflake::new(2)),
                    edited: false,
                    username: Some("testuser".to_string()),
                    display_name: Some("Test User".to_string()),
                    avatar_hash: Some("avatar_hash_png".to_string()),
                    attachment_id: Some((i / 5).try_into().expect("explod")),
                    attachment_filename: Some("test.txt".to_string()),
                    attachment_content_type: Some("text/plain".to_string()),
                })
                .collect::<Vec<_>>()
        };

        records.shuffle(&mut rand::thread_rng());

        let mut messages = Message::from_records(records)
            .expect("expected Ok")
            .into_iter()
            .sorted_by_key(super::Message::id)
            .collect::<Vec<_>>();
        assert!(messages.len() == 5);

        println!("Messages: {messages:#?}");

        for (i, message) in messages.iter_mut().enumerate() {
            assert_eq!(message.id(), Snowflake::new(i as i64));
            assert_eq!(message.channel_id(), Snowflake::new(1));
            assert_eq!(message.content(), Some("Test content"));
            assert!(message.author().is_some());
            assert_eq!(message.author().expect("Should have author").username(), "testuser");
            assert_eq!(
                message.author().expect("Should have author").display_name(),
                Some("Test User")
            );
            assert_eq!(
                message
                    .author()
                    .expect("Should have author")
                    .avatar()
                    .expect("Should have avatar")
                    .avatar_hash(),
                "avatar_hash_png"
            );

            message.attachments.sort_by_key(super::AttachmentLike::id);

            assert_eq!(message.attachments().len(), 5, "message {i} should have 5 attachments",);
            assert_eq!(message.attachments()[0].id(), 0);
            assert_eq!(message.attachments()[1].id(), 1);
            assert_eq!(message.attachments()[2].id(), 2);
            assert_eq!(message.attachments()[3].id(), 3);
            assert_eq!(message.attachments()[4].id(), 4);
            assert_eq!(message.attachments()[0].filename(), "test.txt");
            assert_eq!(message.attachments()[0].mime(), mime::TEXT_PLAIN);
            assert_eq!(message.attachments()[0].channel_id(), Snowflake::new(1));
            assert_eq!(message.attachments()[0].message_id(), Snowflake::new(i as i64));
        }
    }

    #[test]
    fn test_snowflake_conversions() {
        let message = dummy_message();
        let id = Snowflake::<Message>::from(&message);
        assert_eq!(id, Snowflake::new(123));

        let id2 = Snowflake::<Message>::from(message);
        assert_eq!(id2, Snowflake::new(123));
    }
}
