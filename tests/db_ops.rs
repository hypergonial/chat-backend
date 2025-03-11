#![cfg(feature = "db_tests")] // Only runs with `cargo test -F db_tests`

use dotenvy_macro::dotenv;
use sqlx::PgPool;
use utils::fixture_constants::basic::BASIC_USER_1;

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
