//#![cfg(feature = "db_tests")] // Only runs with `cargo test -F db_tests`
#![allow(clippy::unwrap_used, clippy::unreadable_literal)]

use chat_backend::models::{
    channel::{ChannelLike, TextChannel},
    errors::RESTError,
    message::Message,
    omittableoption::OmittableOption,
    request_payloads::UpdateGuild,
    snowflake::Snowflake,
};
use sqlx::PgPool;
use utils::fixture_constants::basic::{BASIC_GUILD_1, BASIC_GUILD_1_GENERAL, BASIC_USER_1}; // add import for channel types

mod utils;

/*
Testing scenario: 'Basic' (Fixture: `basic`)

The 'Basic' scenario is a simple scenario that contains two users and two guilds.

- `BASIC_USER_1` and `BASIC_USER_2` users are in `BASIC_GUILD_1`, but only `BASIC_USER_2` is in `BASIC_GUILD_2`.
- `BASIC_GUILD_1` has the following channels:
  - `BASIC_GUILD_1_GENERAL`
  - `BASIC_GUILD_1_RANDOM`
  - `BASIC_GUILD_1_STAFF`
  - `BASIC_GUILD_1_BOT`
- `BASIC_GUILD_2` has the following channels:
  - `BASIC_GUILD_2_GENERAL`

---

Testing scenario: 'Basic with Messages' (Fixture `basic.messages`)
Dependencies: 'Basic'

The 'Basic with Messages' scenario is an extension of the 'Basic' scenario.
It includes a large number of messages in the `BASIC_GUILD_1_GENERAL` channel.
This scenario is useful for testing message fetching and other operations that involve large numbers of messages.
*/

#[sqlx::test(fixtures("basic"))]
async fn test_user_fetch(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let user = app.ops().fetch_user(BASIC_USER_1).await.expect("DB operation failed");
    assert_eq!(user.id(), BASIC_USER_1);
    assert_eq!(user.username(), "test");
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_and_fetch_read_states(pool: PgPool) {
    let app = utils::DBApp::new(pool);

    // Update with an initial message ID (e.g. 100)
    app.ops()
        .update_read_state(BASIC_USER_1, BASIC_GUILD_1_GENERAL, 100_i64)
        .await
        .unwrap();
    let states = app.ops().fetch_read_states(BASIC_USER_1).await.unwrap();
    let state = states
        .iter()
        .find(|s| s.channel_id == BASIC_GUILD_1_GENERAL)
        .expect("State for channel should exist");
    assert_eq!(state.last_read_message_id.unwrap(), 100_i64.into());

    // Update with a lower message ID; value should remain unchanged (still 100)
    app.ops()
        .update_read_state(BASIC_USER_1, BASIC_GUILD_1_GENERAL, 50_i64)
        .await
        .unwrap();
    let states = app.ops().fetch_read_states(BASIC_USER_1).await.unwrap();
    let state = states
        .iter()
        .find(|s| s.channel_id == BASIC_GUILD_1_GENERAL)
        .expect("State for channel should exist");
    assert_eq!(state.last_read_message_id.unwrap(), 100_i64.into());

    // Update with a higher message ID; value should update (to 150)
    app.ops()
        .update_read_state(BASIC_USER_1, BASIC_GUILD_1_GENERAL, 150_i64)
        .await
        .unwrap();
    let states = app.ops().fetch_read_states(BASIC_USER_1).await.unwrap();
    let state = states
        .iter()
        .find(|s| s.channel_id == BASIC_GUILD_1_GENERAL)
        .expect("State for channel should exist");
    assert_eq!(state.last_read_message_id.unwrap(), 150_i64.into());
}

#[sqlx::test(fixtures("basic"))]
async fn test_is_channel_present(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    // Check for an existing channel and for a non-existent channel.
    let exists = app.ops().is_channel_present(BASIC_GUILD_1_GENERAL).await.unwrap();
    assert!(exists);
    let not_exists = app.ops().is_channel_present(999999_i64).await.unwrap();
    assert!(!not_exists);
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_channel(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let channel = app.ops().fetch_channel(BASIC_GUILD_1_GENERAL).await;
    assert!(channel.is_some());
}

#[sqlx::test(fixtures("basic"))]
async fn test_create_channel(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let existing = app.ops().fetch_channel(BASIC_GUILD_1_GENERAL).await.unwrap();
    let guild = app.ops().fetch_guild(existing.guild_id()).await.unwrap();
    let new_id = Snowflake::gen_new(app.config());
    let test_channel = TextChannel::new(new_id, &guild, "test-channel".to_owned()).into();
    let created = app.ops().create_channel(&test_channel).await.unwrap();
    assert_eq!(created.name(), "test-channel");
    assert_eq!(created.guild_id(), guild.id());
    assert_eq!(created.id(), new_id);
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_channel(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let mut existing = app.ops().fetch_channel(BASIC_GUILD_1_GENERAL).await.unwrap();
    existing.name_mut().clear();
    existing.name_mut().push_str("updated-channel");

    app.ops().update_channel(&existing).await.unwrap();
    let updated = app.ops().fetch_channel(BASIC_GUILD_1_GENERAL).await.unwrap();
    assert_eq!(updated.name(), "updated-channel");
}

#[sqlx::test(fixtures("basic"))]
async fn test_delete_channel(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let existing = app.ops().fetch_channel(BASIC_GUILD_1_GENERAL).await.unwrap();
    app.ops().delete_channel(existing.id()).await.unwrap();
    let exists = app.ops().is_channel_present(existing.id()).await.unwrap();
    assert!(!exists);
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_default(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    // Fetch messages with no before/after/around parameters; should return up to 50 messages
    let messages = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            None,
            None::<Snowflake<Message>>,
            None::<Snowflake<Message>>,
            None::<Snowflake<Message>>,
        )
        .await
        .expect("Failed to fetch messages");
    assert_eq!(messages.len(), 50, "Expected 50 messages");
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_before(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let before_id = 289530032228012033_i64;
    let mut messages = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            Some(10),
            Some(before_id),
            None::<Snowflake<Message>>,
            None::<Snowflake<Message>>,
        )
        .await
        .expect("Failed to fetch messages with 'before'");
    assert!(
        !messages.is_empty(),
        "Messages fetched with 'before' should not be empty"
    );
    for msg in &messages {
        assert!(
            msg.id() < before_id.into(),
            "Message id {} is not less than before_id",
            msg.id()
        );
    }
    messages.sort_by_key(Message::id);
    assert_eq!(messages.first().unwrap().id(), 289530028629299201_i64.into());
    assert_eq!(messages.last().unwrap().id(), 289530032223817729_i64.into());
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_before_partial(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let before_id = 289528069797056514_i64;
    let messages = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            Some(10),
            Some(before_id),
            None::<Snowflake<Message>>,
            None::<Snowflake<Message>>,
        )
        .await
        .expect("Failed to fetch messages with 'before'");
    assert!(
        messages.len() == 5,
        "Messages fetched with 'before' should be exactly 5"
    );
    for msg in messages {
        assert!(
            msg.id() < before_id.into(),
            "Message id {} is not less than before_id",
            msg.id()
        );
    }
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_after(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let after_id = 278891037475344385_i64;
    let mut messages = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            Some(10),
            None::<Snowflake<Message>>,
            Some(after_id),
            None::<Snowflake<Message>>,
        )
        .await
        .expect("Failed to fetch messages with 'after'");
    assert!(
        !messages.is_empty(),
        "Messages fetched with 'after' should not be empty"
    );
    for msg in &messages {
        assert!(
            msg.id() > after_id.into(),
            "Message id {} is not greater than after_id",
            msg.id()
        );
    }
    messages.sort_by_key(Message::id);
    assert_eq!(messages.first().unwrap().id(), 279339971750531073_i64.into());
    assert_eq!(messages.last().unwrap().id(), 289528069834805249_i64.into());
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_after_partial(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let after_id = 289530032198651904_i64;
    let messages = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            Some(10),
            None::<Snowflake<Message>>,
            Some(after_id),
            None::<Snowflake<Message>>,
        )
        .await
        .expect("Failed to fetch messages with 'after'");
    assert!(messages.len() == 5, "Messages fetched with 'after' should be exactly 5");
    for msg in messages {
        assert!(
            msg.id() > after_id.into(),
            "Message id {} is not greater than after_id",
            msg.id()
        );
    }
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_around(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let anchor_id = 278891037475344385_i64;
    let messages = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            Some(10),
            None::<Snowflake<Message>>,
            None::<Snowflake<Message>>,
            Some(anchor_id),
        )
        .await
        .expect("Failed to fetch messages with 'around'");
    assert!(
        !messages.is_empty(),
        "Messages fetched with 'around' should not be empty"
    );
    let contains_anchor = messages.iter().any(|msg| msg.id() == anchor_id.into());
    assert!(contains_anchor, "Anchor message with id {anchor_id} not found");
}

#[sqlx::test(fixtures("basic", "basic_messages"))]
async fn test_fetch_messages_bad_request(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    // Providing both before and after should trigger a bad request error.
    let res = app
        .ops()
        .fetch_messages_from(
            BASIC_GUILD_1_GENERAL,
            Some(10),
            Some(278891037475344385_i64),
            Some(278891037475344385_i64),
            None::<Snowflake<Message>>,
        )
        .await;
    match res {
        Err(RESTError::BadRequest(_)) => {}
        _ => panic!("Expected RESTError::BadRequest when both 'before' and 'after' parameters are provided"),
    }
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_guild(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let guild = app.ops().fetch_guild(BASIC_GUILD_1).await.unwrap();
    assert_eq!(guild.id(), BASIC_GUILD_1);
    assert_eq!(guild.name(), "Test Guild");
    assert_eq!(guild.owner_id(), BASIC_USER_1);
}

#[sqlx::test(fixtures("basic"))]
async fn test_create_guild(pool: PgPool) {
    use chat_backend::models::request_payloads::CreateGuild;
    let app = utils::DBApp::new(pool);
    let payload = CreateGuild {
        name: "Test Guild".to_owned(),
    };
    let (guild, general_channel, owner) = app.ops().create_guild(payload, BASIC_USER_1).await.unwrap();
    assert_eq!(guild.name(), "Test Guild");
    assert_eq!(guild.owner_id(), BASIC_USER_1);
    assert_eq!(general_channel.guild_id(), guild.id());
    assert_eq!(owner.user().id(), BASIC_USER_1);
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_guild(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let guild = app.ops().fetch_guild(BASIC_GUILD_1).await.unwrap();
    let update_payload = UpdateGuild {
        name: Some("Updated Guild".to_owned()),
        owner_id: None,
        avatar: OmittableOption::Omitted,
    };
    let updated = app.ops().update_guild(update_payload, &guild).await.unwrap();
    assert_eq!(updated.name(), "Updated Guild");
    assert_eq!(updated.owner_id(), guild.owner_id());
    assert_eq!(updated.avatar(), guild.avatar());
}

#[sqlx::test(fixtures("basic"))]
async fn test_delete_guild(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    app.ops().delete_guild(BASIC_GUILD_1).await.unwrap();
    let fetched = app.ops().fetch_guild(BASIC_GUILD_1).await;
    assert!(fetched.is_none());
}
