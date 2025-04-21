#![cfg(feature = "db_tests")] // Only runs with `cargo test -F db_tests`
#![allow(clippy::unwrap_used, clippy::unreadable_literal, dead_code, unused_imports)]

use axum::{Router, body::Body};
use chat_backend::main_router;
use http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use utils::{
    app::{RequestBuilderExt, ResponseExt, RouterExt, auth},
    fixture_constants::basic::{BASIC_GUILD_1, BASIC_USER_1},
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

#[sqlx::test(fixtures("basic"))]
async fn get_capabilities(pool: PgPool) {
    let mut router = mock_router(pool).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1")
        .body(Body::empty())
        .unwrap();

    let response = router.push_request(request).await;
    assert_eq!(response.status(), StatusCode::OK);

    let json = response.into_json().await;

    assert_eq!(json["capabilities"].as_u64(), Some(0));
}

#[sqlx::test(fixtures("basic", "basic_credentials"))]
async fn get_user(pool: PgPool) {
    let mut router = mock_router(pool).await;
    let tokens = get_tokens(&mut router).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/users/@me")
        .bearer_auth(tokens.test.clone())
        .body(Body::empty())
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

    let json = response.into_json().await;
    assert_eq!(json, expected);
}

#[sqlx::test(fixtures("basic", "basic_credentials"))]
async fn get_user_2(pool: PgPool) {
    let mut router = mock_router(pool).await;
    let tokens = get_tokens(&mut router).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/users/@me")
        .bearer_auth(tokens.test2.clone())
        .body(Body::empty())
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
    let json = response.into_json().await;
    assert_eq!(json, expected);
}

#[sqlx::test(fixtures("basic", "basic_credentials"))]
async fn fetch_self_guilds(pool: PgPool) {
    let mut router = mock_router(pool).await;
    let tokens = get_tokens(&mut router).await;

    let request = axum::http::Request::builder()
        .method(Method::GET)
        .uri("/api/v1/users/@me/guilds")
        .bearer_auth(tokens.test.clone())
        .body(Body::empty())
        .unwrap();

    let response = router.push_request(request).await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response.into_json().await;

    let expected = json!([
        {
            "id": BASIC_GUILD_1,
            "name": "Test Guild",
            "avatar_hash": null,
            "owner_id": format!("{BASIC_USER_1}"),
        }
    ]);

    assert_eq!(json, expected);
}
