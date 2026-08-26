#!/usr/bin/env bash
# Seed the demo database with sample search results.
# Idempotent: relies on the UNIQUE constraint on search_results.url that is
# declared in live-search/migrations/001_create_search_results.up.sql.  If
# the constraint is missing (e.g. on a pre-migration database) the script
# refuses to TRUNCATE unless `SEED_ALLOW_DESTRUCTIVE=1` is set explicitly.
# Usage: ./scripts/seed-db.sh [DATABASE_URL]
set -euo pipefail

DATABASE_URL="${1:-${DATABASE_URL:-postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo}}"

# Use bash array to keep psql argv separate — prevents shell injection
# from a DATABASE_URL with spaces or metacharacters.
PSQL_ARGS=("$DATABASE_URL" "-t")

echo "Seeding database..."
HAS_UNIQUE_URL=$(psql "${PSQL_ARGS[@]}" -tA -c "
    SELECT 1
    FROM pg_constraint
    WHERE conname = 'search_results_url_key'
    LIMIT 1;
" | tr -d '[:space:]')

if [ -z "$HAS_UNIQUE_URL" ]; then
    echo "  No UNIQUE constraint on search_results.url."
    if [ "${SEED_ALLOW_DESTRUCTIVE:-0}" != "1" ]; then
        echo "  Refusing to TRUNCATE without SEED_ALLOW_DESTRUCTIVE=1." >&2
        echo "  Re-run with: SEED_ALLOW_DESTRUCTIVE=1 $0" >&2
        exit 2
    fi
    echo "  SEED_ALLOW_DESTRUCTIVE=1 set — TRUNCATE-ing table first."
    psql "${PSQL_ARGS[@]}" -c "TRUNCATE search_results;" >/dev/null
    ON_CONFLICT_CLAUSE=""
else
    ON_CONFLICT_CLAUSE="ON CONFLICT (url) DO NOTHING"
fi

psql "${PSQL_ARGS[@]}" <<SQL
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
psql "${PSQL_ARGS[@]}" -c "SELECT title FROM search_results WHERE fts @@ plainto_tsquery('english', 'rust') ORDER BY created_at DESC LIMIT 5;"