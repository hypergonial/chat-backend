#![allow(clippy::unwrap_used, clippy::unreadable_literal)]

use axum::{Router, response::Response};
use chat_backend::main_router;
use http::{Method, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use utils::{
    app::{RequestBuilderExt, RouterExt, auth},
    mock_app,
};

mod utils;

static TOKENS: OnceCell<Tokens> = OnceCell::const_new();

/// Tokens for authenticating test requests
struct Tokens {
    /// Token for the user "test"
    test: String,
    /// Token for the user "test2"
    test2: String,
}

/// Get the tokens for the test users.
/// The tokens will only be requested once, and the tokens will be cached
/// for future use.
async fn get_tokens(router: &mut Router) -> &Tokens {
    TOKENS
        .get_or_init(async || Tokens {
            test: auth(router, "dGVzdDpBbW9uZ3VzMS4=".to_string()).await,
            test2: auth(router, "dGVzdDI6QW1vbmd1czEu".to_string()).await,
        })
        .await
}

async fn mock_router(pool: PgPool) -> Router {
    let app = mock_app(pool).await;
    main_router(app)
}

async fn assert_eq_json(response: Response, expected: serde_json::Value) {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();

    assert_eq!(json, expected);
}

#[sqlx::test(fixtures("basic"))]
async fn get_capabilities(pool: PgPool) {
    let mut router = mock_router(pool).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = router.push_request(request).await;
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq_json(response, json!({"capabilities": 0})).await;
}

#[sqlx::test(fixtures("basic", "basic_credentials"))]
async fn get_user(pool: PgPool) {
    let mut router = mock_router(pool).await;
    let tokens = get_tokens(&mut router).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/users/@me")
        .bearer_auth(tokens.test.clone())
        .body(axum::body::Body::empty())
        .unwrap();

    let response = router.push_request(request).await;

    assert_eq!(response.status(), StatusCode::OK);

    let expected = json!({
        "id": "274560698946818049",
        "username": "test",
        "display_name": null,
        "avatar_hash": null,
        "presence": null
    });

    assert_eq_json(response, expected).await;
}

#[sqlx::test(fixtures("basic", "basic_credentials"))]
async fn get_user_2(pool: PgPool) {
    let mut router = mock_router(pool).await;
    let tokens = get_tokens(&mut router).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/users/@me")
        .bearer_auth(tokens.test2.clone())
        .body(axum::body::Body::empty())
        .unwrap();

    let response = router.push_request(request).await;

    assert_eq!(response.status(), StatusCode::OK);

    let expected = json!({
        "id": "278890683744522241",
        "username": "test2",
        "display_name": null,
        "avatar_hash": null,
        "presence": null
    });

    assert_eq_json(response, expected).await;
}
