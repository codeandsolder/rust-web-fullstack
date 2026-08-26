DROP INDEX IF EXISTS search_results_event_seq_idx;
ALTER TABLE search_results DROP COLUMN IF EXISTS event_seq;
