-- Add UNIQUE constraint to fcm_tokens table
ALTER TABLE fcm_tokens
ADD CONSTRAINT unq_fcm_tokens_token UNIQUE token;