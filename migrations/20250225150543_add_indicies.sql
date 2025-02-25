-- Some indicies to speed up common queries that were not covered by the primary keys
-- Querying messages in a given channel is quite common
CREATE INDEX idx_message_channel_id ON messages USING HASH (channel_id);
-- Querying all channels of a guild is needed when a new client connects
CREATE INDEX idx_channel_guild_id ON channels USING HASH (guild_id);
-- For queries filtering by user_id (finding all guilds a user belongs to)
CREATE INDEX idx_members_user_id ON members USING HASH (user_id);
-- For queries filtering by guild_id (finding all members of a guild)
CREATE INDEX idx_members_guild_id ON members USING HASH (guild_id);
-- For message fetching, all attachment data is needed
CREATE INDEX idx_attachments_message_id ON attachments USING HASH (message_id);