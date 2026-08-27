-- Add a UNIQUE constraint on search_results.url so that the seed script's
-- `ON CONFLICT (url) DO NOTHING` is valid (PostgreSQL requires an explicit
-- unique or exclusion target for ON CONFLICT).
--
-- Idempotent: PostgreSQL does NOT support `IF NOT EXISTS` on `ADD CONSTRAINT`,
-- so we wrap the ALTER in a DO block that checks pg_constraint first. This
-- makes the migration safe to re-run when the migration row in
-- `_sqlx_migrations` is missing (e.g. the migration runner binary was killed
-- after the ALTER succeeded but before the row was committed).

DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'search_results_url_key'
    ) THEN
        ALTER TABLE search_results
            ADD CONSTRAINT search_results_url_key UNIQUE (url);
    END IF;
END $$;