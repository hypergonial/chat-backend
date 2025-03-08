use std::{collections::HashMap, sync::Arc, time::Duration};

use gcp_auth;
use http::StatusCode;
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

/// See: <https://firebase.google.com/docs/cloud-messaging/auth-server#use-credentials-to-mint-access-tokens>
static FCM_SCOPES: [&str; 1] = ["https://www.googleapis.com/auth/firebase.messaging"];

#[derive(Serialize)]
struct FirebaseMessage<'a> {
    token: &'a str,
    notification: &'a Notification,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a HashMap<String, String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// An implementation of Firebase Cloud Messaging (FCM) for sending push notifications.
pub struct FirebaseMessaging {
    provider: Arc<dyn gcp_auth::TokenProvider>,
    project_id: String,
    http: Arc<reqwest::Client>,
}

#[derive(Debug, Error)]
pub enum FirebaseError {
    #[error("Failed to authenticate with notification service: {0}")]
    Auth(#[from] gcp_auth::Error),
    #[error("Failed to send push notification: {0}")]
    Request(#[from] reqwest::Error),
}

impl FirebaseMessaging {
    /// Create a new instance of the FCM client.
    ///
    /// # Panics
    ///
    /// If the TLS backend cannot be initialized,
    /// or the HTTP client resolver cannot load the system configuration.
    ///
    /// # Errors
    ///
    /// If no GCP auth provider was resolved.
    /// This is typically due to a missing `GOOGLE_APPLICATION_CREDENTIALS` environment variable.
    pub async fn new() -> Result<Self, FirebaseError> {
        let provider = gcp_auth::provider().await?;
        let project_id = provider.project_id().await?.to_string();

        Ok(Self {
            provider,
            project_id,
            http: Arc::new(
                reqwest::Client::builder()
                    .use_rustls_tls()
                    .http2_prior_knowledge()
                    .build()
                    .expect("Failed to create HTTP client for FCM"),
            ),
        })
    }

    /// Get a new access token for the FCM API.
    ///
    /// Calling this function multiple times will not result in multiple requests, as
    /// the token is cached and refreshed automatically when it expires.
    ///
    /// # Errors
    ///
    /// If the token cannot be fetched.
    async fn get_token(&self) -> Result<Arc<gcp_auth::Token>, gcp_auth::Error> {
        self.provider.token(&FCM_SCOPES).await
    }

    /// Helper function to determine if an error is retriable
    fn is_retriable_error(err: &reqwest::Error) -> bool {
        if let Some(status) = err.status() {
            if status.is_server_error() {
                return true;
            }

            matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::TOO_EARLY | StatusCode::REQUEST_TIMEOUT
            )
        } else {
            err.is_timeout() || err.is_connect()
        }
    }

    async fn perform_send(
        http: &reqwest::Client,
        auth_token: impl Into<&str>,
        project_id: impl Into<&str>,
        to: impl Into<&str>,
        notification: &Notification,
        data: Option<&HashMap<String, String>>,
    ) -> Result<(), FirebaseError> {
        let auth_token = auth_token.into();
        let project_id = project_id.into();
        let message = FirebaseMessage {
            token: to.into(),
            notification,
            data,
        };

        let mut attempts = 0;
        let max_attempts = 5; // Maximum retry attempts
        let base_delay = Duration::from_millis(100); // Starting delay (100ms)

        loop {
            attempts += 1;

            let result = http
                .post(format!(
                    "https://fcm.googleapis.com/v1/projects/{project_id}/messages:send",
                ))
                .header("Authorization", format!("Bearer {auth_token}"))
                .json(&json!({ "message": message }))
                .send()
                .await;

            let mut retry_after = None;

            match result {
                Ok(response) => {
                    retry_after = response
                        .headers()
                        .get("Retry-After")
                        .and_then(|value| value.to_str().ok().and_then(|value| value.parse::<u64>().ok()));

                    match response.error_for_status() {
                        Ok(_) => return Ok(()),
                        Err(err) => {
                            if attempts >= max_attempts || !Self::is_retriable_error(&err) {
                                return Err(FirebaseError::Request(err));
                            }
                        }
                    }
                }
                Err(err) => {
                    if attempts >= max_attempts || !Self::is_retriable_error(&err) {
                        return Err(FirebaseError::Request(err));
                    }
                }
            }

            // Try respecting the Retry-After header, falling back to exponential backoff
            let delay = retry_after.map_or_else(|| base_delay * 2u32.pow(attempts - 1), Duration::from_secs);
            let jitter = rand::random::<f32>() * 0.5; // Add up to 50% jitter
            let delay_with_jitter = delay.mul_f32(1.0 + jitter);

            tracing::debug!(
                "FCM messages:send retry {}/{} after {:?}",
                attempts,
                max_attempts,
                delay_with_jitter
            );

            tokio::time::sleep(delay_with_jitter).await;
        }
    }

    /// Send a push notification to a device.
    ///
    /// # Arguments
    ///
    /// * `token` - The device token to send the notification to
    /// * `notification` - The notification to send
    /// * `data` - Additional data to send with the notification
    ///
    /// # Errors
    ///
    /// If the notification could not be sent.
    pub async fn send_notification(
        &self,
        token: impl Into<&str>,
        notification: Notification,
        data: Option<HashMap<String, String>>,
    ) -> Result<(), FirebaseError> {
        Self::perform_send(
            &self.http,
            self.get_token().await?.as_str(),
            &*self.project_id,
            token,
            &notification,
            data.as_ref(),
        )
        .await?;

        Ok(())
    }

    /// Send a push notification to multiple devices.
    ///
    /// This function will send notifications to each device in parallel.
    ///
    /// # Arguments
    ///
    /// * `tokens` - A list of device tokens to send the notification to
    /// * `notification` - The notification to send
    /// * `data` - Additional data to send with the notification
    ///
    /// # Errors
    ///
    /// If any notification could not be sent.
    pub async fn send_notification_to_multiple(
        &self,
        tokens: Vec<impl Into<String>>,
        notification: Notification,
        data: Option<HashMap<String, String>>,
    ) -> Result<(), FirebaseError> {
        // Spawn n tasks for each token
        let auth_token: Arc<str> = Arc::from(self.get_token().await?.as_str());
        let project_id: Arc<str> = Arc::from(self.project_id.clone());
        let notification: Arc<Notification> = Arc::new(notification);
        let data: Option<Arc<HashMap<String, String>>> = data.map(Arc::new);

        let tasks = tokens.into_iter().map(|token| {
            let auth_token = auth_token.clone();
            let project_id = project_id.clone();
            let notification = notification.clone();
            let data = data.clone();
            let token = token.into();
            let http = self.http.clone();

            tokio::spawn(async move {
                Self::perform_send(
                    &http,
                    &*auth_token,
                    &*project_id,
                    &*token,
                    &notification,
                    data.as_deref(),
                )
                .await
            })
        });

        // Wait for all tasks to complete
        let results = futures::future::join_all(tasks).await;

        // Check if any task failed

        for result in results {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => return Err(err),
                Err(err) => assert!(!err.is_panic(), "Task panicked: {err:?}"),
            }
        }

        Ok(())
    }
}
