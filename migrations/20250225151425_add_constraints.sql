-- Clear invalid states
DELETE FROM guilds
WHERE char_length(name) < 3
    OR char_length(name) > 32;
DELETE FROM channels
WHERE char_length(name) < 3
    OR char_length(name) > 32;
-- Add constraints for channel names
ALTER TABLE channels
ADD CONSTRAINT channel_name_in_bounds CHECK (
        char_length(name) BETWEEN 3 AND 32
    );
-- Add constraints for guild names
ALTER TABLE guilds
ADD CONSTRAINT guild_name_in_bounds CHECK (
        char_length(name) BETWEEN 3 AND 32
    );