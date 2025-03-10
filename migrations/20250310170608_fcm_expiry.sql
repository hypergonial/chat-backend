-- Add last_refresh to fcm_tokens
ALTER TABLE fcm_tokens
ADD COLUMN last_refresh TIMESTAMPTZ NOT NULL DEFAULT NOW();