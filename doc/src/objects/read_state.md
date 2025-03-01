# Read State

## Overview

A read state is a pairing of a channel & message ID, representing the last message that the current user has read in that channel. This is used to determine the last message that the user has read in a channel, and is used to determine the unread state of a channel.

## Fields

| Field | Type | Description |
| --- | --- | --- |
| `channel_id` | `Snowflake` | The ID of the channel that the read state is for. |
| `last_read_message_id` | `Snowflake` | The ID of the last message that the user has read in the channel. |
| `last_message_id` | `Snowflake?` | The ID of the last message in the channel, if any. |

**Caution!** The `last_message_id` field is nullable, and may not be present in all read states, as it is possible (albeit rare) that all messages in a channel have since been deleted.

## Example Payload

```json
{
    "channel_id": "123456789123456789",
    "last_read_message_id": "123456789123456789",
    "last_message_id": "123456789123456789"
}
```
