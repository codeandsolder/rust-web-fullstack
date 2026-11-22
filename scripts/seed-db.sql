-- Requires the migrated schema, including the unique URL constraint.
-- Safe to repeat; existing rows are preserved.
INSERT INTO search_results (title, url, snippet) VALUES
  ('Rust Programming Language', 'https://www.rust-lang.org/', 'A language empowering everyone to build reliable and efficient software.'),
  ('Leptos Full-Stack Web Framework', 'https://leptos.dev/', 'Build modern web applications with fine-grained reactivity.'),
  ('PostgreSQL Documentation', 'https://www.postgresql.org/docs/', 'Powerful, open source object-relational database system.'),
  ('Axum Web Framework', 'https://docs.rs/axum/', 'Modular web framework built with Tokio, Tower, and Hyper.'),
  ('sqlx Rust SQL Toolkit', 'https://github.com/launchbadge/sqlx', 'The Rust SQL Toolkit. Compile-time checked queries.')
ON CONFLICT (url) DO NOTHING;
