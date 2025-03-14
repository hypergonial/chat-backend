# /guilds

## POST

### Summary

Creates a guild. Dispatches the [GUILD_CREATE](../gateway/events.md#guild_create) gateway event.

### Payload

```json
{
    "name": "Among Us",
}
```

### Response

The created [Guild](../objects/guild.md) object.

# /guilds/\{guild_id\}

## GET

### Summary

Gets a guild's data.

### Response

A [Guild](../objects/guild.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | You are not authorized to view this resource. |
| 404  | The guild was not found. |

## PATCH

### Summary

Update a guild. All fields are optional. All fields specified will be overridden. Dispatches the [GUILD_UPDATE](../gateway/events.md#guild_update) gateway event.

Note that if you edit the owner of the guild, you will lose permissions to make further edits to it.

### Example Payload

```json
{
    "name": "Among Us",
    "avatar": "data:image/jpeg;base64,/9j/4AAQSkZJRgABAgAAZABkAAD",
    "owner_id": null,
}
```

### Response

The updated [Guild](../objects/guild.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | You are not authorized to patch this resource. |
| 404  | The guild was not found. |

## DELETE

### Summary

Deletes a guild. Dispatches the [GUILD_REMOVE](../gateway/events.md#guild_remove) gateway event.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | You are not authorized to delete this resource. |
| 404  | The guild was not found. |

# /guilds/\{guild_id\}/channels

## POST

### Summary

Creates a channel in a guild. Dispatches the [CHANNEL_CREATE](../gateway/events.md#channel_create) gateway event.

### Example Payload

```json
{
    "type": "GUILD_TEXT", // Currently only this channel-type is supported
    "name": "channel-name",
}
```

### Response

The created [Channel](../objects/channel.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 403  | You are not authorized to create this resource. |
| 404  | The guild was not found. |

# /guilds/\{guild_id\}/members

## POST

### Summary

Adds the currently authenticated user as a member to a guild. If the member is already in the guild, this will simply return the member's data. Dispatches the [MEMBER_CREATE](../gateway/events.md#member_create) gateway event.

### Response

The created [Member](../objects/member.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 404  | The guild was not found. |

# /guilds/\{guild_id\}/members/\{user_id\}

## GET

### Summary

Gets a member's data. Use `@me` as the `user_id` to get the authenticated user's data.

### Response

A [Member](../objects/member.md) object.

### Errors

| Code | Description |
| ---- | ----------- |
| 404  | The member or guild was not found. |

## DELETE

### Summary

Removes a member from a guild. Dispatches the [MEMBER_REMOVE](../gateway/events.md#member_remove) gateway event.

> Note: This endpoint currently only supports the use of `@me` as the `user_id`.

### Response

An empty response.

### Errors

| Code | Description |
| ---- | ----------- |
| 404  | The member or guild was not found. |
