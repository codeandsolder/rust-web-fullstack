#!/usr/bin/env bash
# Seed the demo database with sample search results.
# Idempotent: relies on the UNIQUE constraint on search_results.url that is
# declared in live-search/migrations/001_create_search_results.up.sql.  If
# the constraint is missing (e.g. on a pre-migration database) the script
# falls back to TRUNCATE-then-INSERT so it still succeeds.
# Usage: ./scripts/seed-db.sh [DATABASE_URL]
set -euo pipefail
DATABASE_URL="${1:-postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo}"
PSQL_CMD="psql $DATABASE_URL -t"
echo "Seeding database..."
# Detect whether the unique constraint on url exists.  If not, TRUNCATE
# first so re-running the script doesn't accumulate duplicate rows.
HAS_UNIQUE_URL=$($PSQL_CMD -tA -c "
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'search_results_url_key'
    LIMIT 1;
" | tr -d '[:space:]')
if [ -z "$HAS_UNIQUE_URL" ]; then
    echo "  No UNIQUE constraint on search_results.url — TRUNCATE-ing table first."
    $PSQL_CMD -c "TRUNCATE search_results;" >/dev/null
    ON_CONFLICT_CLAUSE=""
else
    ON_CONFLICT_CLAUSE="ON CONFLICT (url) DO NOTHING"
fi
$PSQL_CMD <<SQL
INSERT INTO search_results (title, url, snippet) VALUES
  ('Rust Programming Language', 'https://www.rust-lang.org/', 'A language empowering everyone to build reliable and efficient software.'),
  ('Leptos Full-Stack Web Framework', 'https://leptos.dev/', 'Build modern web applications with fine-grained reactivity.'),
  ('PostgreSQL Documentation', 'https://www.postgresql.org/docs/', 'Powerful, open source object-relational database system.'),
  ('Axum Web Framework', 'https://docs.rs/axum/', 'Modular web framework built with Tokio, Tower, and Hyper.'),
  ('sqlx Rust SQL Toolkit', 'https://github.com/launchbadge/sqlx', 'The Rust SQL Toolkit. Compile-time checked queries.')
$ON_CONFLICT_CLAUSE;
SQL
echo "Inserted 5 sample search results (skipped duplicates)."
echo "Testing full-text search:"
$PSQL_CMD -c "SELECT title FROM search_results WHERE fts @@ plainto_tsquery('english', 'rust') ORDER BY created_at DESC LIMIT 5;"
