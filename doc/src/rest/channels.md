# /channels

# /channels/\{channel_id\}

## GET

### Summary

Gets a channel's data.

### Response

A [Channel](../objects/channel.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | The user is not in the guild the channel is located in. |
| 404  | The channel was not found. |

## DELETE

### Summary

Deletes a channel. Dispatches the [CHANNEL_REMOVE](../gateway/events.md#channel_remove) gateway event.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | The user has no permission to delete the channel. |
| 404  | The channel was not found. |

# /channels/\{channel_id\}/messages

## GET

### Summary

Fetch a sequence of messages from a channel.

### Query Parameters

| Name | Type | Description |
| ---- | ---- | ----------- |
| before | snowflake? | Get messages before this message ID. |
| after | snowflake? | Get messages after this message ID. |
| around | snowflake? | Get messages around this message ID. The message belonging to this ID will also be included, if it still exists. |
| limit | integer? | The maximum number of messages to return. Capped at 100, defaults to 50. |

**Note:** Only one of `before`, `after`, or `around` can be specified. If none are specified, the endpoint will return the most recent messages in the given channel.

### Response

An array of [Message](../objects/message.md) objects.

**Note:** The ordering of messages returned by this endpoint is not guaranteed.

## POST

### Summary

Sends a message to a channel. Dispatches the [MESSAGE_CREATE](../gateway/events.md#message_create) gateway event to all guild members.

### Payload

This endpoint expects a `multipart/form-data` payload. The following fields are supported:

| Name | Type | Description |
| ---- | ---- | ----------- |
| json | application/json | Valid json that represents the message's textually representable information |
| attachment-0..9 | Any valid MIME | A file to attach to the message. The `filename` field is mandatory. |

> Note: While both `json` and `attachment` are optional, at least one of them **must** be present.

Example:

```http
POST /channels/123/messages HTTP/1.1
Content-Type: multipart/form-data; boundary=--------------------------1234567890

----------------------------1234567890
Content-Disposition: form-data; name="json"
Content-Type: application/json

{
    "content": "Hello, world!",
    "nonce": "catch me catch me catch me catch.."
}
----------------------------1234567890
Content-Disposition: form-data; name="attachment-0"; filename="cat.png"
Content-Type: image/png

<cat.png bytes>
----------------------------1234567890
Content-Disposition: form-data; name="attachment-1"; filename="dog.gif"
Content-Type: image/gif

<dog.gif bytes>
```

### Response

The created [Message](../objects/message.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | The user is not in the guild the channel is located in. |
| 404  | The channel was not found. |

# /channels/\{channel_id\}/messages/\{message_id\}

## PATCH

### Summary

Updates a message with new data. All fields are optional, and all fields specified will overwrite the current values. Dispatches the [MESSAGE_UPDATE](../gateway/events.md#message_update) gateway event to all guild members.

### Payload

| Name | Type | Description |
| ---- | ---- | ----------- |
| content | string? | The new contents of the message. |

### Response

The updated [Message](../objects/message.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | The user has no permission to update the message. |
| 404  | The message or channel was not found. |

## DELETE

### Summary

Deletes the message. Dispatches the [MESSAGE_REMOVE](../gateway/events.md#message_remove) gateway event to all guild members.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | The user has no permission to delete the message. |
| 404  | The message or channel was not found. |

# /channels/\{channel_id\}/messages/\{message_id\}/ack

## POST

### Summary

Acknowledges a message. Dispatches the [MESSAGE_ACK](../gateway/events.md#message_ack) gateway event to all sessions of the currently authenticated user.

### Errors

| Code | Description |
| ---- | ----------- |
| 404  | The channel was not found. |
| 403  | The user is not in the guild the channel is located in. |
