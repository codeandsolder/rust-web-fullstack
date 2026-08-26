-- A refresh token digest identifies exactly one issued credential. Random
-- collisions are already negligible; the database constraint also prevents
-- accidental duplicate rows from making replay lookup ambiguous.
CREATE UNIQUE INDEX IF NOT EXISTS refresh_tokens_hashed_token_uq
    ON refresh_tokens (hashed_token);
