-- Reverse of 002_add_url_unique_constraint.up.sql.

ALTER TABLE search_results
    DROP CONSTRAINT IF EXISTS search_results_url_key;