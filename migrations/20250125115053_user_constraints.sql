-- Ensure usernames fall within a certain length
DELETE FROM users
WHERE char_length(username) < 3
    OR char_length(username) > 32;
DELETE FROM users
WHERE display_name IS NOT NULL
    AND char_length(display_name) < 3;
ALTER TABLE users
ADD CONSTRAINT username_in_bounds CHECK (
        char_length(username) BETWEEN 3 AND 32
    ),
    ADD CONSTRAINT display_name_in_bounds CHECK (
        display_name IS NULL
        OR char_length(display_name) >= 3
    );