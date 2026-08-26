CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS search_results (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title       TEXT NOT NULL,
    url         TEXT NOT NULL,
    snippet     TEXT NOT NULL DEFAULT '',
    fts         TSVECTOR,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- GIN index for full-text search on the fts column
CREATE INDEX IF NOT EXISTS idx_search_results_fts ON search_results USING GIN(fts);

-- Trigger function that emits a NOTIFY carrying only the row id.
--
-- **The payload is intentionally tiny.** PostgreSQL's default NOTIFY
-- payload limit is 8000 bytes; an AFTER INSERT trigger that puts
-- `title`, `url`, and `snippet` into the payload can fail the
-- transaction if any single row exceeds that bound. The consumer
-- (live-search/src/db.rs::forward_notification) fetches the full row
-- by id when it receives the notification, so the broadcast event
-- still carries the full typed payload.
CREATE OR REPLACE FUNCTION notify_search_result()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('search_results',
        json_build_object('id', NEW.id)::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_search_result_insert'
          AND tgrelid = 'search_results'::regclass
    ) THEN
        CREATE TRIGGER trg_search_result_insert
        AFTER INSERT ON search_results
        FOR EACH ROW
        EXECUTE FUNCTION notify_search_result();
    END IF;
END;
$$;

-- Trigger function that auto-populates the fts tsvector column
-- from title and snippet on insert or update.
CREATE OR REPLACE FUNCTION auto_update_fts()
RETURNS TRIGGER AS $$
BEGIN
    NEW.fts := to_tsvector('english', COALESCE(NEW.title, '') || ' ' || COALESCE(NEW.snippet, ''));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'trg_search_result_fts'
          AND tgrelid = 'search_results'::regclass
    ) THEN
        CREATE TRIGGER trg_search_result_fts
        BEFORE INSERT OR UPDATE OF title, snippet ON search_results
        FOR EACH ROW
        EXECUTE FUNCTION auto_update_fts();
    END IF;
END;
$$;
