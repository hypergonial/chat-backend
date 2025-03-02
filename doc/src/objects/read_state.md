# Read State

## Overview

A read state is a pairing of a channel & message ID, representing the last message that the current user has read in that channel. This is used to determine the last message that the user has read in a channel, and is used to determine the unread state of a channel.

## Fields

| Field | Type | Description |
| --- | --- | --- |
| `channel_id` | `Snowflake` | The ID of the channel that the read state is for. |
| `last_read_message_id` | `Snowflake?` | The ID of the last message that the user has read in the channel. |
| `last_message_id` | `Snowflake?` | The ID of the last message in the channel, if any. |

**Caution!** Both the `last_message_id` and `last_read_message_id` fields are nullable, and may not be present in all read states. If the `last_message_id` is not present, the channel is considered to be empty. If `last_read_message_id` is not present, the user does not have a read state in the channel.

## Example Payload

```json
{
    "channel_id": "123456789123456789",
    "last_read_message_id": "123456789123456789",
    "last_message_id": "123456789123456789"
}
```
