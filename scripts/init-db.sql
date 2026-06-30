-- Initial database bootstrap. Runs on first container start via /docker-entrypoint-initdb.d/.
-- Extensions only. Application schema is owned by sqlx migrations.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
CREATE EXTENSION IF NOT EXISTS pgcrypto;
