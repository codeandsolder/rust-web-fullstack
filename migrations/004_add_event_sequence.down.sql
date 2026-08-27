DROP TRIGGER IF EXISTS trg_search_result_event_seq ON search_results;
DROP FUNCTION IF EXISTS assign_search_result_event_seq();
DROP INDEX IF EXISTS search_results_event_seq_idx;
ALTER TABLE search_results DROP COLUMN IF EXISTS event_seq;
DROP SEQUENCE IF EXISTS search_results_event_seq_seq;
