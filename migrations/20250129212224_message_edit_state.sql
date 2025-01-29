-- Add migration script here
ALTER TABLE messages
ADD COLUMN edited BOOLEAN DEFAULT FALSE NOT NULL;