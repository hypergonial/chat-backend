use std::sync::{Arc, Weak};

use aws_sdk_s3::{
    Client,
    error::SdkError,
    operation::head_bucket::HeadBucketError,
    primitives::ByteStream,
    types::{Delete, Object, ObjectIdentifier},
};
use bytes::{Bytes, BytesMut};
use mime::Mime;

use super::{
    channel::Channel, errors::AppError, guild::Guild, message::Message, snowflake::Snowflake, state::ApplicationState,
};

pub type S3Client = Client;

const ALLOW_ALL_DOWNLOADS_POLICY: &str = r#"{
    "Version": "2012-10-17",
    "Statement": [
        {
            "Sid": "AllowAnonymousRead",
            "Action": ["s3:GetObject"],
            "Effect": "Allow",
            "Principal": "*",
            "Resource": ["arn:aws:s3:::{bucketName}/*"]
        }
    ]
}"#;

/// All S3 buckets used by the application.
#[derive(Debug, Clone)]
pub struct S3Service {
    app: Weak<ApplicationState>,
    client: S3Client,
}

impl S3Service {
    /// Create all buckets from the given config.
    pub const fn new(client: S3Client) -> Self {
        Self {
            client,
            app: Weak::new(),
        }
    }

    pub fn bind_to(&mut self, app: Weak<ApplicationState>) {
        self.app = app;
    }

    pub fn app(&self) -> Arc<ApplicationState> {
        self.app.upgrade().expect("Application state has been dropped.")
    }

    pub const fn client(&self) -> &S3Client {
        &self.client
    }

    pub const fn get_bucket(&self, name: &'static str) -> Bucket {
        Bucket::new(self, name)
    }

    /// The attachments bucket.
    /// It is responsible for storing all message attachments.
    pub const fn attachments(&self) -> Bucket {
        self.get_bucket("attachments")
    }

    pub const fn users(&self) -> Bucket {
        self.get_bucket("users")
    }

    pub const fn guilds(&self) -> Bucket {
        self.get_bucket("guilds")
    }

    fn get_policy_string(bucket: &str) -> String {
        ALLOW_ALL_DOWNLOADS_POLICY.replace("{bucketName}", bucket)
    }

    /// Create all buckets if they do not exist.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn create_buckets(&self) -> Result<(), AppError> {
        for bucket in [self.attachments().name(), self.users().name(), self.guilds().name()] {
            match self.client.head_bucket().bucket(bucket).send().await {
                Ok(_) => {
                    tracing::info!("S3 Bucket {} already exists, skipping creation.", bucket);
                }
                Err(SdkError::ServiceError(e)) if matches!(e.err(), HeadBucketError::NotFound(_)) => {
                    self.client.create_bucket().bucket(bucket).send().await?;
                    self.client
                        .put_bucket_policy()
                        .bucket(bucket)
                        .policy(Self::get_policy_string(bucket))
                        .send()
                        .await?;

                    tracing::info!("Created S3 bucket: {}", bucket);
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    /// Remove all S3 data for the given message.
    ///
    /// ## Arguments
    ///
    /// * `channel` - The channel the message is in.
    /// * `message` - The message to remove all data for.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn remove_all_for_message(
        &self,
        channel: impl Into<Snowflake<Channel>>,
        message: impl Into<Snowflake<Message>>,
    ) -> Result<(), AppError> {
        let bucket = self.attachments();
        let channel_id: Snowflake<Channel> = channel.into();
        let message_id: Snowflake<Message> = message.into();
        let attachments = bucket.list_objects(format!("{channel_id}/{message_id}"), None).await?;

        if attachments.is_empty() {
            return Ok(());
        }

        bucket
            .delete_objects(
                attachments
                    .into_iter()
                    .map(|o| o.key.unwrap_or_else(|| message_id.to_string()))
                    .collect(),
            )
            .await
    }

    /// Remove all S3 data for the given channel.
    ///
    /// ## Arguments
    ///
    /// * `channel` - The channel to remove all data for.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn remove_all_for_channel(&self, channel: impl Into<Snowflake<Channel>>) -> Result<(), AppError> {
        let bucket = self.attachments();
        let channel_id: Snowflake<Channel> = channel.into();
        let attachments = bucket.list_objects(channel_id.to_string(), None).await?;

        if attachments.is_empty() {
            return Ok(());
        }

        bucket
            .delete_objects(
                attachments
                    .into_iter()
                    .map(|o| o.key.unwrap_or_else(|| channel_id.to_string()))
                    .collect(),
            )
            .await
    }

    /// Remove all S3 data for the given guild.
    ///
    /// ## Arguments
    ///
    /// * `guild` - The guild to remove all data for.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn remove_all_for_guild(&self, guild: impl Into<Snowflake<Guild>>) -> Result<(), AppError> {
        let guild_id: i64 = guild.into().into();

        let channel_ids: Vec<i64> = sqlx::query!("SELECT id FROM channels WHERE guild_id = $1", guild_id)
            .fetch_all(self.app().db())
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect();

        for channel_id in channel_ids {
            self.remove_all_for_channel(channel_id).await?;
        }

        Ok(())
    }
}

/// An abstraction for S3 buckets.
#[derive(Clone, Debug)]
pub struct Bucket<'a> {
    name: &'static str,
    buckets: &'a S3Service,
}

impl<'a> Bucket<'a> {
    pub const fn new(buckets: &'a S3Service, name: &'static str) -> Self {
        Self { name, buckets }
    }

    /// The name of this bucket.
    pub const fn name(&self) -> &str {
        self.name
    }

    /// Fetch an object from this bucket.
    ///
    /// ## Arguments
    ///
    /// * `client` - The S3 client to use.
    /// * `key` - The key of the object to fetch.
    ///
    /// ## Returns
    ///
    /// [`Bytes`] - The object data.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn get_object(&self, key: impl Into<String>) -> Result<Bytes, AppError> {
        let mut resp = self
            .buckets
            .client()
            .get_object()
            .bucket(self.name)
            .key(key)
            .send()
            .await?;

        let mut bytes = BytesMut::new();
        while let Some(chunk) = resp.body.next().await {
            bytes.extend_from_slice(&chunk.expect("Failed to read S3 object chunk"));
        }

        Ok(bytes.freeze())
    }

    /// Upload an object to this bucket.
    ///
    /// ## Arguments
    ///
    /// * `client` - The S3 client to use.
    /// * `key` - The key of the object to upload.
    /// * `data` - The data to upload.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn put_object(
        &self,
        key: impl Into<String>,
        data: impl Into<ByteStream>,
        content_type: &Mime,
    ) -> Result<(), AppError> {
        self.buckets
            .client()
            .put_object()
            .bucket(self.name)
            .content_type(content_type.to_string())
            .key(key)
            .body(data.into())
            .send()
            .await?;

        Ok(())
    }

    /// List objects in this bucket.
    ///
    /// ## Arguments
    ///
    /// * `client` - The S3 client to use.
    /// * `prefix` - The prefix to filter by.
    /// * `limit` - The maximum number of objects to fetch.
    ///
    /// ## Returns
    ///
    /// [`Vec<Object>`] - The objects fetched.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn list_objects(&self, prefix: impl Into<String>, limit: Option<i32>) -> Result<Vec<Object>, AppError> {
        let mut objects = Vec::new();

        // AWS-SDK has a nice pagination API to send continuation tokens implicitly, so we use that
        let mut req = self.buckets.client().list_objects_v2().bucket(self.name).prefix(prefix);

        if let Some(limit) = limit {
            req = req.max_keys(limit);
        }

        let mut paginator = req.into_paginator().send();

        while let Some(resp) = paginator.next().await {
            if let Some(contents) = resp?.contents {
                objects.extend(contents);
            }
        }

        Ok(objects)
    }

    /// Delete an object from this bucket.
    ///
    /// ## Arguments
    ///
    /// * `client` - The S3 client to use.
    /// * `key` - The key of the object to delete.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn delete_object(&self, key: impl Into<String>) -> Result<(), AppError> {
        self.buckets
            .client()
            .delete_object()
            .bucket(self.name)
            .key(key)
            .send()
            .await?;

        Ok(())
    }

    /// Delete multiple objects from this bucket.
    ///
    /// ## Arguments
    ///
    /// * `client` - The S3 client to use.
    /// * `keys` - The keys of the objects to delete.
    ///
    /// ## Errors
    ///
    /// * [`AppError::S3`] - If the S3 request fails.
    pub async fn delete_objects(&self, keys: Vec<impl Into<String>>) -> Result<(), AppError> {
        let objects: Vec<ObjectIdentifier> = keys
            .into_iter()
            .map(|k| {
                ObjectIdentifier::builder()
                    .set_key(Some(k.into()))
                    .build()
                    .expect("Failed to build ObjectIdentifier")
            })
            .collect();

        self.buckets
            .client()
            .delete_objects()
            .bucket(self.name)
            .delete(
                Delete::builder()
                    .set_objects(Some(objects))
                    .build()
                    .expect("Failed to build Delete"),
            )
            .send()
            .await?;

        Ok(())
    }
}
