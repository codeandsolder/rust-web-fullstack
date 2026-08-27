CREATE TABLE IF NOT EXISTS refresh_tokens (
    jti          UUID        PRIMARY KEY,
    -- A "family" of refresh tokens that share lineage. The first row uses
    -- its own jti as family_id; subsequent rotations reuse the original
    -- family_id so the server can revoke the whole chain when an
    -- already-rotated token is replayed (see `gateway::auth::refresh`).
    family_id    UUID        NOT NULL,
    subject      UUID        NOT NULL,
    hashed_token BYTEA       NOT NULL,
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,
    replaced_by  UUID,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Lookup index for refresh-token rotation: client presents token, server
-- hashes it, server looks up by hashed_token. Without this index, every
-- refresh is a sequential scan. BYTEA equality is indexed natively by
-- Postgres B-tree.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_hashed_token
    ON refresh_tokens (hashed_token);

-- Family-revocation index: revoking a whole family is a single UPDATE.
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_family_id
    ON refresh_tokens (family_id);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_subject
    ON refresh_tokens (subject);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at
    ON refresh_tokens (expires_at);