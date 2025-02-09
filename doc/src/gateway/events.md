# Events

All events follow the following format:

```json
{
    "event": "EVENT_NAME",
    "data": {
        "field": "value",
        "another_field": "another_value"
    }
}
```

In the following descriptions, when talking about the `data` field, it is implied that the event is wrapped in an object with an `event` field, as shown above.

## MESSAGE_CREATE

### Summary

Sent when a message is sent in a channel that the currently authenticated user is a member of.

### Data

A [Message](../objects/message.md) object.

## MESSAGE_UPDATE

### Summary

Sent when a message is updated in a channel that the currently authenticated user is a member of.

### Data

A [Message](../objects/message.md) object.

## MESSAGE_REMOVE

### Summary

Sent when a message is removed in a channel that the currently authenticated user is a member of.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `id` | `Snowflake` | The ID of the message that was removed. |
| `channel_id` | `Snowflake` | The channel's ID the message was part of. |
| `guild_id` | `Snowflake` | The guild's ID the message was part of. |

## MEMBER_CREATE

### Summary

Sent when a member joins a guild that the currently authenticated user is a member of.

### Data

A [Member](../objects/member.md) object.

## MEMBER_REMOVE

### Summary

Sent when a member leaves a guild that the currently authenticated user is a member of.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `id` | `Snowflake` | The member's ID that left the guild. |
| `guild_id` | `Snowflake` | The guild's ID. |

## USER_UPDATE

### Summary

Sent when a user that the currently authenticated user shares a guild with updates their data.

### Data

A [User](../objects/user.md) object representing the updated user.

## GUILD_CREATE

### Summary

Sent when a guild is created or on initial connection. The client is expected to cache the guild member & channel data sent in this event, and update it accordingly when receiving associated events.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `guild` | [`Guild`](../objects/guild.md) | The guild's data. |
| `members` | [`Member[]`](../objects/member.md) | The guild's members. |
| `channels` | [`Channel[]`](../objects/channel.md) | The guild's channels. |

## GUILD_UPDATE

### Summary

Sent when a guild is updated.

### Data

A [Guild](../objects/guild.md) object representing the updated guild.

## GUILD_REMOVE

### Summary

A [`Guild`](../objects/guild.md) guild object representing the guild that was deleted.

### Data

The ID of the guild that was deleted.

## CHANNEL_CREATE

### Summary

Sent when a channel is created.

### Data

A [Channel](../objects/channel.md) object representing the channel that was created.

## CHANNEL_REMOVE

### Summary

Sent when a channel is deleted.

### Data

A [Channel](../objects/channel.md) object representing the channel that was deleted.

## HELLO

### Summary

Sent as the first event by the server after connecting, including the **heartbeat interval** the client should use for this session.

### Data

The entire `data` field consists of a single integer, specifying the heartbeat interval in **milliseconds**.

## READY

### Summary

Sent when the client has successfully authenticated and the server is ready to send events.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `user` | [`User`](../objects/user.md) | The client's user data. |
| `guilds` | [`Guild[]`](../objects/guild.md) | The guilds the client is a member of. |

## INVALID_SESSION

### Summary

Sent when the client's session is invalidated. The client is expected to reconnect and send a new `IDENTIFY` payload. The websocket connection is terminated after this event is sent.

### Data

A `String` containing the reason for the session invalidation.
