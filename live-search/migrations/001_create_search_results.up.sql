CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE search_results (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title       TEXT NOT NULL,
    url         TEXT NOT NULL,
    snippet     TEXT NOT NULL DEFAULT '',
    fts         TSVECTOR,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- GIN index for full-text search on the fts column
CREATE INDEX idx_search_results_fts ON search_results USING GIN(fts);

-- Trigger function that sends a JSON notification via pg_notify
-- whenever a new row is inserted.
CREATE OR REPLACE FUNCTION notify_search_result()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('search_results', json_build_object(
        'type',    'SearchResult',
        'title',   NEW.title,
        'url',     NEW.url,
        'snippet', NEW.snippet
    )::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_search_result_insert
AFTER INSERT ON search_results
FOR EACH ROW
EXECUTE FUNCTION notify_search_result();

-- Trigger function that auto-populates the fts tsvector column
-- from title and snippet on insert or update.
CREATE OR REPLACE FUNCTION auto_update_fts()
RETURNS TRIGGER AS $$
BEGIN
    NEW.fts := to_tsvector('english', COALESCE(NEW.title, '') || ' ' || COALESCE(NEW.snippet, ''));
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_search_result_fts
BEFORE INSERT OR UPDATE OF title, snippet ON search_results
FOR EACH ROW
EXECUTE FUNCTION auto_update_fts();
