#![allow(dead_code, clippy::unreadable_literal)]

use chat_backend::models::{channel::Channel, guild::Guild, snowflake::Snowflake, user::User};

/// Fixture constants for the `basic` fixture.
pub mod basic {
    use super::*;

    /// The 'test' user from the basic fixture.
    pub const BASIC_USER_1: Snowflake<User> = Snowflake::new(274560698946818049);

    /// The 'test2' user from the basic fixture.
    pub const BASIC_USER_2: Snowflake<User> = Snowflake::new(278890683744522241);

    /// The 'Test Guild' from the basic fixture.
    pub const BASIC_GUILD_1: Snowflake<Guild> = Snowflake::new(274586748720386049);

    /// The 'general' channel for 'Test Guild' from the basic fixture.
    /// This channel will contain thousands of messages if the basic.messages fixture is also applied.
    pub const BASIC_GUILD_1_GENERAL: Snowflake<Channel> = BASIC_GUILD_1.cast();

    /// The 'random' channel for 'Test Guild' from the basic fixture.
    pub const BASIC_GUILD_1_RANDOM: Snowflake<Channel> = Snowflake::new(275245989999677441);

    /// The 'staff' channel for 'Test Guild' from the basic fixture.
    pub const BASIC_GUILD_1_STAFF: Snowflake<Channel> = Snowflake::new(275246119536562177);

    /// The 'bot' channel for 'Test Guild' from the basic fixture.
    pub const BASIC_GUILD_1_BOT: Snowflake<Channel> = Snowflake::new(282978799527137281);

    /// The 'Test Guild 2' from the basic fixture.
    pub const BASIC_GUILD_2: Snowflake<Guild> = Snowflake::new(278890858219180033);

    /// The 'general' channel for 'Test Guild 2' from the basic fixture.
    pub const BASIC_GUILD_2_GENERAL: Snowflake<Channel> = BASIC_GUILD_2.cast();
}

// Add additional modules for other fixtures here, as needed.
