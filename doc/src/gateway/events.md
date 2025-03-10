# Events

This page documents events clients may receive through the gateway, documenting when they occur and what data they contain.

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

## HELLO

### Summary

Sent as the first event by the server after connecting, including the **heartbeat interval** the client should use for this session.

### Data

The entire `data` field consists of a single integer, specifying the heartbeat interval in **milliseconds**.

## READY

### Summary

Sent when the client has successfully authenticated and the server is ready to send events. A sequence of [`GUILD_CREATE`](#guild_create) events will follow this event, containing more extensive information about each guild the user is a member of.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `user` | [`User`](../objects/user.md) | The client's user data. |
| `guilds` | [`Guild[]`](../objects/guild.md) | The guilds the client is a member of. |
| `read_states` | [`ReadState[]`](../objects/read_state.md) | The user's read states for each channel. |

## HEARTBEAT_ACK

### Summary

Sent by the server in response to a [`HEARTBEAT`](./requests.md#heartbeat) request.
If the client does not receive this event within ~5 seconds, it should assume the connection is dead and proceed to reconnect.

### Data

This event contains no data.

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

## MESSAGE_ACK

### Summary

Sent when a session belonging to the currently authenticated user acknowledges a message. This can be used to synchronize read states between multiple connected sessions.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `channel_id` | `Snowflake` | The ID of the channel the message was acknowledged in. |
| `message_id` | `Snowflake` | The ID of the message that was acknowledged. |


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

Sent when a guild is deleted.

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

## TYPING_START

### Summary

Sent when a user starts typing in a given channel. Clients may use this event to show a typing indicator.
The typing indicator should be shown for a maximum of 6 seconds after the last `TYPING_START` event is received for the given user.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `user_id` | `Snowflake` | The user that started typing. |
| `channel_id` | `Snowflake` | The channel the user started typing in. |
