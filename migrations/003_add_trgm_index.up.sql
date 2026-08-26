-- Trigram index for typo-tolerant fuzzy search on title.
-- Extension is created here (not just in init-db.sql) so testcontainers-based
-- fresh Postgres instances also work.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX IF NOT EXISTS idx_search_results_title_trgm
  ON search_results USING gin (title gin_trgm_ops);