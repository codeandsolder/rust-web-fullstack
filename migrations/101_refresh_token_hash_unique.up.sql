-- A refresh token digest identifies exactly one issued credential. Random
-- collisions are already negligible; the database constraint also prevents
-- accidental duplicate rows from making replay lookup ambiguous.
CREATE UNIQUE INDEX refresh_tokens_hashed_token_uq
    ON refresh_tokens (hashed_token);

-- The unique B-tree also serves equality lookups, so the original non-unique
-- index from migration 100 would only add write/storage overhead.
DROP INDEX IF EXISTS idx_refresh_tokens_hashed_token;
