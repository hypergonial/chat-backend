//#![cfg(feature = "db_tests")] // Only runs with `cargo test -F db_tests`
#![allow(clippy::unwrap_used, clippy::unreadable_literal)]

use chat_backend::models::{
    channel::{ChannelLike, TextChannel},
    errors::RESTError,
    member::UserLike,
    message::Message,
    omittableoption::OmittableOption,
    request_payloads::{CreateGuild, UpdateGuild, UpdateMessage, UpdateUser},
    snowflake::Snowflake,
};
use sqlx::PgPool;
use utils::fixture_constants::basic::{
    BASIC_GUILD_1, BASIC_GUILD_1_BOT, BASIC_GUILD_1_GENERAL, BASIC_GUILD_1_RANDOM, BASIC_GUILD_1_STAFF, BASIC_GUILD_2,
    BASIC_USER_1, BASIC_USER_2,
}; // add import for channel types

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

    let channels = app.ops().fetch_channels_for(BASIC_GUILD_1).await.unwrap();
    assert!(channels.is_empty(), "Channels should be deleted with the guild");

    let members = app.ops().fetch_members_for(BASIC_GUILD_1).await.unwrap();
    assert!(members.is_empty(), "Members should be deleted with the guild");

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
        .unwrap();
    assert!(messages.is_empty(), "Messages should be deleted with the guild");
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_guild_owner(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let guild = app.ops().fetch_guild(BASIC_GUILD_1).await.unwrap();
    let owner = app.ops().fetch_guild_owner(&guild).await.unwrap();
    assert_eq!(owner.user().id(), guild.owner_id());
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_members_for(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let members = app.ops().fetch_members_for(BASIC_GUILD_1).await.unwrap();
    assert!(members.len() >= 2, "Expected at least two members in the guild");
    let member_ids: Vec<_> = members.into_iter().map(|m| m.user().id()).collect();
    assert!(member_ids.contains(&BASIC_USER_1), "BASIC_USER_1 not found in members");
    assert!(member_ids.contains(&BASIC_USER_2), "BASIC_USER_2 not found in members");
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_channels_for(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let channels = app.ops().fetch_channels_for(BASIC_GUILD_1).await.unwrap();
    assert_eq!(channels.len(), 4, "Expected 4 channels in the guild");
    assert!(
        channels.iter().any(|c| c.id() == BASIC_GUILD_1_GENERAL),
        "Expected general channel in channels"
    );
    assert!(
        channels.iter().any(|c| c.id() == BASIC_GUILD_1_RANDOM),
        "Expected random channel in channels"
    );
    assert!(
        channels.iter().any(|c| c.id() == BASIC_GUILD_1_STAFF),
        "Expected staff channel in channels"
    );
    assert!(
        channels.iter().any(|c| c.id() == BASIC_GUILD_1_BOT),
        "Expected bot channel in channels"
    );
}

#[sqlx::test(fixtures("basic"))]
async fn test_create_and_fetch_member(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let (guild, _channel, _owner) = app
        .ops()
        .create_guild(
            CreateGuild {
                name: "Member Test Guild".to_owned(),
            },
            BASIC_USER_1,
        )
        .await
        .unwrap();
    let member = app.ops().fetch_member(BASIC_USER_2, guild.id()).await.unwrap();
    assert!(member.is_none(), "BASIC_USER_2 should not be a member initially");
    let new_member = app.ops().create_member(guild.id(), BASIC_USER_2).await.unwrap();
    assert_eq!(new_member.user().id(), BASIC_USER_2);
    let fetched = app.ops().fetch_member(BASIC_USER_2, guild.id()).await.unwrap();
    assert!(fetched.is_some(), "Member should be found after creation");
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_member_nickname(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let (guild, _channel, _owner) = app
        .ops()
        .create_guild(
            CreateGuild {
                name: "Nickname Test Guild".to_owned(),
            },
            BASIC_USER_1,
        )
        .await
        .unwrap();
    let mut member = app.ops().create_member(guild.id(), BASIC_USER_2).await.unwrap();
    member.nickname_mut().replace("CoolNickname".to_owned());
    app.ops().update_member(&member).await.unwrap();
    let updated = app.ops().fetch_member(BASIC_USER_2, guild.id()).await.unwrap().unwrap();
    assert_eq!(updated.nickname().as_deref(), Some("CoolNickname"));
}

#[sqlx::test(fixtures("basic"))]
async fn test_has_member_function(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let has_user1 = app.ops().has_member(BASIC_GUILD_1, BASIC_USER_1).await.unwrap();
    assert!(has_user1, "BASIC_USER_1 should be a member of BASIC_GUILD_1");
    let has_user2 = app.ops().has_member(BASIC_GUILD_1, BASIC_USER_2).await.unwrap();
    assert!(has_user2, "BASIC_USER_2 should be a member of BASIC_GUILD_1");
    let has_user_1_in_guild_2 = app.ops().has_member(BASIC_GUILD_2, BASIC_USER_1).await.unwrap();
    assert!(
        !has_user_1_in_guild_2,
        "BASIC_USER_1 should not be a member of BASIC_GUILD_2"
    );
}

#[sqlx::test(fixtures("basic"))]
async fn test_delete_member_non_owner(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let (guild, _channel, _owner) = app
        .ops()
        .create_guild(
            CreateGuild {
                name: "DeleteMember Guild".to_owned(),
            },
            BASIC_USER_1,
        )
        .await
        .unwrap();
    app.ops().create_member(guild.id(), BASIC_USER_2).await.unwrap();
    let member = app.ops().fetch_member(BASIC_USER_2, guild.id()).await.unwrap();
    assert!(member.is_some(), "Member should exist before deletion");
    app.ops().delete_member(&guild, BASIC_USER_2).await.unwrap();
    let after_delete = app.ops().fetch_member(BASIC_USER_2, guild.id()).await.unwrap();
    assert!(after_delete.is_none(), "Member should be deleted");
}

#[sqlx::test(fixtures("basic"))]
async fn test_delete_member_owner_error(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let guild = app.ops().fetch_guild(BASIC_GUILD_1).await.unwrap();
    let res = app.ops().delete_member(&guild, BASIC_USER_1).await;
    match res {
        Err(RESTError::Forbidden(_)) => { /* expected */ }
        _ => panic!("Deleting guild owner should return a Forbidden error"),
    }
}

#[sqlx::test(fixtures("basic"))]
async fn test_commit_and_fetch_message(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let msg_id = Snowflake::gen_new(app.config());

    let author = app.ops().fetch_user(BASIC_USER_1).await.expect("fetch_user failed");

    let message = Message::builder()
        .id(msg_id)
        .author(UserLike::User(author))
        .channel_id(BASIC_GUILD_1_GENERAL)
        .content(Some("Test commit message".into()))
        .build()
        .expect("Failed to build message");

    app.ops().commit_message(&message).await.expect("commit_message failed");

    let fetched = app.ops().fetch_message(msg_id).await.expect("fetch_message failed");
    assert!(fetched.is_some(), "Message should exist after commit");
    let fetched_msg = fetched.unwrap();
    assert_eq!(fetched_msg.id(), msg_id);
    assert_eq!(fetched_msg.content(), Some("Test commit message"));
    assert_eq!(fetched_msg.author().map(UserLike::id), Some(BASIC_USER_1));
    assert!(fetched_msg.attachments().is_empty());
    assert_eq!(fetched_msg.channel_id(), BASIC_GUILD_1_GENERAL);
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_message(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let msg_id = Snowflake::gen_new(app.config());

    let author = app.ops().fetch_user(BASIC_USER_1).await.expect("fetch_user failed");

    let message = Message::builder()
        .id(msg_id)
        .author(UserLike::User(author))
        .channel_id(BASIC_GUILD_1_GENERAL)
        .content(None)
        .build()
        .expect("Failed to build message");

    app.ops().commit_message(&message).await.expect("commit_message failed");

    let update_payload = UpdateMessage {
        content: OmittableOption::Some("Updated content".to_owned()),
    };
    let updated_msg = app
        .ops()
        .update_message(msg_id, update_payload)
        .await
        .expect("update_message failed");
    assert_eq!(updated_msg.content(), Some("Updated content"));
}

#[sqlx::test(fixtures("basic"))]
async fn test_delete_message(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let msg_id = Snowflake::gen_new(app.config());

    let author = app.ops().fetch_user(BASIC_USER_1).await.expect("fetch_user failed");

    let message = Message::builder()
        .id(msg_id)
        .author(UserLike::User(author))
        .channel_id(BASIC_GUILD_1_GENERAL)
        .content(None)
        .build()
        .expect("Failed to build message");

    app.ops().commit_message(&message).await.expect("commit_message failed");

    app.ops()
        .delete_message(BASIC_GUILD_1_GENERAL, msg_id)
        .await
        .expect("delete_message failed");
    let fetched = app.ops().fetch_message(msg_id).await.expect("fetch_message failed");
    assert!(fetched.is_none(), "Message should be deleted");
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_presence(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let presence = app.ops().fetch_presence(BASIC_USER_1).await;
    assert!(presence.is_some(), "Presence should exist for BASIC_USER_1");

    let not_existing = app.ops().fetch_presence(999999_i64).await;
    assert!(
        not_existing.is_none(),
        "Presence should not exist for non-existent user"
    );
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_user_by_username(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let user = app.ops().fetch_user_by_username("test").await;
    assert!(user.is_some(), "User with username 'test' should exist");
    assert_eq!(
        user.unwrap().id(),
        BASIC_USER_1,
        "The fetched user should be BASIC_USER_1"
    );
}

#[sqlx::test(fixtures("basic"))]
async fn test_is_username_taken(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let exists = app.ops().is_username_taken("test").await.unwrap();
    assert!(exists, "'test' should be a taken username");
    let not_exists = app.ops().is_username_taken("nonexistentusername").await.unwrap();
    assert!(!not_exists, "'nonexistentusername' should not be taken");
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_guilds_for_function(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let guilds_user1 = app.ops().fetch_guilds_for(BASIC_USER_1).await.unwrap();
    assert_eq!(guilds_user1.len(), 1);
    assert_eq!(guilds_user1[0].id(), BASIC_GUILD_1);
    assert_eq!(guilds_user1[0].name(), "Test Guild");

    let guilds_user2 = app.ops().fetch_guilds_for(BASIC_USER_2).await.unwrap();
    assert_eq!(guilds_user2.len(), 2);
    let guild_ids: Vec<_> = guilds_user2.into_iter().map(|g| g.id()).collect();
    assert!(guild_ids.contains(&BASIC_GUILD_1));
    assert!(guild_ids.contains(&BASIC_GUILD_2));
}

#[sqlx::test(fixtures("basic"))]
async fn test_fetch_guild_ids_for_function(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let ids_user1 = app.ops().fetch_guild_ids_for(BASIC_USER_1).await.unwrap();
    assert_eq!(ids_user1.len(), 1);
    assert!(ids_user1.contains(&BASIC_GUILD_1));

    let ids_user2 = app.ops().fetch_guild_ids_for(BASIC_USER_2).await.unwrap();
    assert_eq!(ids_user2.len(), 2);
    assert!(ids_user2.contains(&BASIC_GUILD_1));
    assert!(ids_user2.contains(&BASIC_GUILD_2));
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_user_success(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let new_username = "updated_test";
    let new_display_name = "Updated Test";
    let payload = UpdateUser {
        username: Some(new_username.to_owned()),
        display_name: OmittableOption::Some(new_display_name.to_owned()),
        avatar: OmittableOption::Omitted,
    };
    let updated = app.ops().update_user(BASIC_USER_1, payload).await.unwrap();
    assert_eq!(updated.username(), new_username);
    assert_eq!(updated.display_name(), Some(new_display_name));
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_user_no_change(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let old_user = app.ops().fetch_user(BASIC_USER_1).await.unwrap();
    let payload = UpdateUser {
        username: None,
        display_name: OmittableOption::Omitted,
        avatar: OmittableOption::Omitted,
    };
    let updated = app.ops().update_user(BASIC_USER_1, payload).await.unwrap();
    // ...existing code...
    assert_eq!(updated.username(), old_user.username());
    assert_eq!(updated.display_name(), old_user.display_name());
}

#[sqlx::test(fixtures("basic"))]
async fn test_update_user_nonexistent(pool: PgPool) {
    let app = utils::DBApp::new(pool);
    let payload = UpdateUser {
        username: Some("nonexistent".to_string()),
        display_name: OmittableOption::Omitted,
        avatar: OmittableOption::Omitted,
    };
    let result = app.ops().update_user(999999_i64, payload).await;
    match result {
        Err(RESTError::NotFound(_)) => { /* expected */ }
        _ => panic!("Expected NotFound error for non-existent user"),
    }
}
