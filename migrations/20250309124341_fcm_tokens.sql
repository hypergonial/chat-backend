-- Add fcm_tokens to store the FCM tokens of a user
CREATE TABLE fcm_tokens (
    user_id BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token TEXT NOT NULL,
    PRIMARY KEY (user_id, token)
);
-- Reads are almost always done by user
CREATE INDEX idx_fcm_tokens_user_id ON fcm_tokens USING HASH (user_id);