-- Create two test users
INSERT INTO users (id, username)
VALUES (274560698946818049, 'test'),
    (278890683744522241, 'test2');
-- Create a guild owned by 'test'
INSERT INTO guilds (id, owner_id, name)
VALUES (
        274586748720386049,
        274560698946818049,
        'Test Guild'
    );
-- Create channels in the guild
INSERT INTO channels(id, channel_type, guild_id, name)
VALUES (
        274586748720386049,
        'TEXT_CHANNEL',
        274586748720386049,
        'general'
    ),
    (
        275245989999677441,
        'TEXT_CHANNEL',
        274586748720386049,
        'random'
    ),
    (
        275246119536562177,
        'TEXT_CHANNEL',
        274586748720386049,
        'staff'
    ),
    (
        282978799527137281,
        'TEXT_CHANNEL',
        274586748720386049,
        'bot'
    );
-- Add member entries to guild
INSERT INTO members (user_id, guild_id, joined_at)
VALUES (274560698946818049, 274586748720386049, 0),
    (278890683744522241, 274586748720386049, 0);
-- Create a second guild owned by test2
INSERT INTO guilds (id, owner_id, name)
VALUES (
        278890858219180033,
        278890683744522241,
        'Test Guild 2'
    );
-- Create a channel in the second guild
INSERT INTO channels(id, channel_type, guild_id, name)
VALUES (
        278890858219180033,
        'TEXT_CHANNEL',
        278890858219180033,
        'general'
    );
-- Only add test2 to the second guild
INSERT INTO members (user_id, guild_id, joined_at)
VALUES (278890683744522241, 278890858219180033, 0);