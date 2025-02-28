# Read State

## Overview

A read state is a pairing of a channel & message ID, representing the last message that the current user has read in that channel. This is used to determine the last message that the user has read in a channel, and is used to determine the unread state of a channel.

## Fields

| Field | Type | Description |
| --- | --- | --- |
| `channel_id` | `Snowflake` | The ID of the channel that the read state is for. |
| `message_id` | `Snowflake` | The ID of the last message that the user has read in the channel. |

## Example Payload

```json
{
    "channel_id": "123456789123456789",
    "message_id": "123456789123456789"
}
```
