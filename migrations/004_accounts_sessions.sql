CREATE TABLE accounts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username TEXT NOT NULL CONSTRAINT accounts_username_unique UNIQUE
        CHECK (username = lower(username) AND username ~ '^[a-z0-9][a-z0-9_.-]{2,63}$'),
    password_hash TEXT NOT NULL CHECK (
        password_hash LIKE '$argon2id$v=19$%' AND char_length(password_hash) BETWEEN 40 AND 512),
    disabled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE sessions (
    token_digest BYTEA PRIMARY KEY CHECK (octet_length(token_digest) = 32),
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '7 days'),
    CHECK (expires_at > created_at)
);
CREATE INDEX sessions_account ON sessions(account_id);
CREATE INDEX sessions_expiry ON sessions(expires_at);
