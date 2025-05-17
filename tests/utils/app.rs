use std::net::SocketAddr;

use axum::{Router, body::Body, extract::Request, response::Response};
use chat_backend::{
    app::{App, ApplicationState, Config},
    external::Database,
    gateway::Gateway,
};
use http::{Method, StatusCode};
use http_body_util::BodyExt;
use secrecy::Secret;
use sqlx::PgPool;
use tower::{Service, ServiceExt};

pub async fn mock_app(pool: PgPool) -> App {
    let db = Database::from_pool(pool);
    let config = Config::builder()
        .database_url(Secret::new(String::new()))
        .s3(None)
        .listen_addr("127.0.0.1:8080".parse::<SocketAddr>().expect("Not valid SocketAddr"))
        .machine_id(0)
        .process_id(0)
        .app_secret(Secret::new(String::from("test")))
        .build()
        .expect("Failed to build Config");

    ApplicationState::from_components(db, Gateway::new(), config, None, None)
        .await
        .expect("Failed to create ApplicationState")
}

/// Get a token for the given credentials.
///
/// # Arguments
///
/// * `router` - The router to use.
/// * `authorization` - The base64 encoded authorization header for Basic authentication.
///
/// # Returns
///
/// Returns the token as a string.
pub async fn auth(router: &mut Router, authorization: String) -> String {
    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/users/auth")
        .header("Authorization", format!("Basic {authorization}"))
        .body(axum::body::Body::empty())
        .unwrap();

    let response = router.push_request(request).await;

    assert_eq!(response.status(), StatusCode::OK);

    let json = response.into_json().await;

    json["token"].as_str().expect("Token should be a string").to_string()
}

pub trait RequestBuilderExt {
    fn bearer_auth(self, token: impl Into<String>) -> Self;
}

impl RequestBuilderExt for http::request::Builder {
    fn bearer_auth(self, token: impl Into<String>) -> Self {
        self.header("Authorization", format!("Bearer {}", token.into()))
    }
}

pub trait ResponseExt {
    async fn into_json(self) -> serde_json::Value;
}

impl ResponseExt for Response<Body> {
    async fn into_json(self) -> serde_json::Value {
        let bytes = self
            .into_body()
            .collect()
            .await
            .expect("Failed to convert body into Bytes")
            .to_bytes();

        serde_json::from_slice::<serde_json::Value>(&bytes).expect("Failed to deserialize body to JSON")
    }
}

pub trait RouterExt {
    async fn push_request(&mut self, request: Request) -> Response;
}

impl RouterExt for Router {
    async fn push_request(&mut self, request: Request) -> Response {
        ServiceExt::<Request>::ready(self)
            .await
            .expect("Failed to ready router")
            .call(request)
            .await
            .expect("Failed to call router")
    }
}
