CREATE TABLE IF NOT EXISTS refresh_tokens (
    jti          UUID        PRIMARY KEY,
    subject      UUID        NOT NULL,
    hashed_token BYTEA       NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Lookup index for refresh-token rotation: client presents token, server hashes
-- it, server looks up by hashed_token. Without this index, every refresh is a
-- sequential scan. BYTEA equality is indexed natively by Postgres B-tree.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_hashed_token
    ON refresh_tokens (hashed_token);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_subject
    ON refresh_tokens (subject);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at
    ON refresh_tokens (expires_at);