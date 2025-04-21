# Testing Scenarios

The testing scenarios are designed to provide a variety of environments and configurations for testing the functionality of the application.

These scenarios include different user setups, guilds, channels, and messages to ensure comprehensive testing coverage.

The constants referenced in this document are defined in `utils::fixture_constants`.

## Basic (Fixture: `basic`)

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

## Basic with Messages (Fixture `basic_messages`)

Dependencies: Basic

The 'Basic with Messages' scenario is an extension of the 'Basic' scenario.
It includes a large number of messages in the `BASIC_GUILD_1_GENERAL` channel.
This scenario is useful for testing message fetching and other operations that involve large numbers of messages.

---

## Testing scenario: Basic with Credentials (Fixture `basic_credentials`)

Dependencies: Basic

Adds login credentials for `BASIC_USER_1` and `BASIC_USER_2`.
- `BASIC_USER_1` has the following credentials:
  - `username`: "test"
  - `password`: "Amongus1."

- `BASIC_USER_2` has the following credentials:
    - `username`: "test2"
    - `password`: "Amongus1."
