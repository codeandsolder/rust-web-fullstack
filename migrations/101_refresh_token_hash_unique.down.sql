CREATE INDEX IF NOT EXISTS idx_refresh_tokens_hashed_token
    ON refresh_tokens (hashed_token);
DROP INDEX IF EXISTS refresh_tokens_hashed_token_uq;
