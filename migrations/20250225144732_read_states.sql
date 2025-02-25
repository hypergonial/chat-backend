-- Add read_states to store the last read message by a user
CREATE TABLE read_states (
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel_id BIGINT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    message_id BIGINT NOT NULL,
    PRIMARY KEY (user_id, channel_id)
);
-- Reads are almost always done by user
CREATE INDEX idx_read_states_user_id ON read_states USING HASH (user_id);
-- Writes are almost always done by (user, channel)
CREATE INDEX idx_read_states_channel_id ON read_states USING HASH (channel_id);