-- This runs on first container start via /docker-entrypoint-initdb.d/

CREATE TABLE IF NOT EXISTS search_results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    snippet TEXT NOT NULL,
    fts tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(snippet, '')), 'B')
    ) STORED,
    created_at TIMESTAMPTZ DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_search_fts ON search_results USING GIN(fts);

CREATE OR REPLACE FUNCTION notify_search_result()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('search_results', row_to_json(NEW)::text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS on_search_result_insert ON search_results;
CREATE TRIGGER on_search_result_insert
    AFTER INSERT ON search_results
    FOR EACH ROW EXECUTE FUNCTION notify_search_result();
