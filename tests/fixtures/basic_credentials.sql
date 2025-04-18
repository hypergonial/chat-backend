-- Add login credentials to the test users
INSERT INTO secrets (user_id, password, is_valid, last_changed)
VALUES (
        274560698946818049,
        -- Hash+salt of 'Amongus1.'
        '$argon2id$v=19$m=19456,t=2,p=1$qHB7XPxn7ioNpfMgMM2zVA$D5uuHyOGpn2UPSoNlAnECpnnDcpmEwI+iQ8hAzCLxn0',
        true,
        0
    ),
    (
        278890683744522241,
        -- Hash+salt of 'Amongus1.'
        '$argon2id$v=19$m=19456,t=2,p=1$qHB7XPxn7ioNpfMgMM2zVA$D5uuHyOGpn2UPSoNlAnECpnnDcpmEwI+iQ8hAzCLxn0',
        true,
        0
    );