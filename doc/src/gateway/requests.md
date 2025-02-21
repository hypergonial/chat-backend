# Requests

This page documents requests clients can send through the gateway.
Sending requests that are not documented here may result in unexpected behavior,
most likely resulting in an immediate disconnection from the gateway.

All requests follow the following format:

```json
{
    "event": "REQUEST_TYPE",
    "data": {
        "field": "value",
        "another_field": "another_value"
    }
}
```

In the following descriptions, when talking about the `data` field, it is implied that the request is wrapped in an object with an `event` field, as shown above.

## IDENTIFY

### Summary

Sent when a client wants to identify itself to the gateway. This must be the first request sent by the client, sent right after receiving the [`HELLO`](events.md#hello) event.
If this is not sent within 5 seconds of connecting, the gateway will close the connection.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `token` | `string` | The client's authentication token. |

## HEARTBEAT

### Summary

Sent when a client wants to keep the connection alive. The gateway will respond with a [`HEARTBEAT_ACK`](events.md#heartbeat_ack) event if the connection is still alive.
Clients should assume the connection is dead if no response is received within ~5 seconds and proceed to reconnect.

### Data

This request contains no data, and the `data` field should be omitted.

## START_TYPING

### Summary

Used to set a typing indicator in a given channel. Triggers a [`TYPING_START`](events.md#typing_start) event for all clients that can access the channel.
If clients want to maintain the typing indicator, they should send this request at least once every <6 seconds,
since clients are expected to dismiss the typing indicator if no request is received within that time frame.

### Data

| Field | Type | Description |
| --- | --- | --- |
| `channel_id` | `Snowflake` | The channel's ID the client wants to set a typing indicator on. |