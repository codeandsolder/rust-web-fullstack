#!/usr/bin/env bash
# Seed the demo database with sample search results.
# Idempotent: uses ON CONFLICT (url) DO NOTHING so it's safe to run repeatedly.
# Usage: ./scripts/seed-db.sh [DATABASE_URL]
set -euo pipefail
DATABASE_URL="${1:-postgres://rwf:rwf_dev_password@localhost:5432/rwf_demo}"
PSQL_CMD="psql $DATABASE_URL -t"
echo "Seeding database..."
$PSQL_CMD <<'SQL'
INSERT INTO search_results (title, url, snippet) VALUES
  ('Rust Programming Language', 'https://www.rust-lang.org/', 'A language empowering everyone to build reliable and efficient software.'),
  ('Leptos Full-Stack Web Framework', 'https://leptos.dev/', 'Build modern web applications with fine-grained reactivity.'),
  ('PostgreSQL Documentation', 'https://www.postgresql.org/docs/', 'Powerful, open source object-relational database system.'),
  ('Axum Web Framework', 'https://docs.rs/axum/', 'Modular web framework built with Tokio, Tower, and Hyper.'),
  ('sqlx Rust SQL Toolkit', 'https://github.com/launchbadge/sqlx', 'The Rust SQL Toolkit. Compile-time checked queries.')
ON CONFLICT (url) DO NOTHING;
SQL
echo "Inserted 5 sample search results (skipped duplicates)."
echo "Testing full-text search:"
$PSQL_CMD -c "SELECT title FROM search_results WHERE fts @@ plainto_tsquery('english', 'rust') ORDER BY created_at DESC LIMIT 5;"
