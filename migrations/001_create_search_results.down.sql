DROP TRIGGER IF EXISTS trg_search_result_insert ON search_results;
DROP TRIGGER IF EXISTS trg_search_result_fts ON search_results;
DROP FUNCTION IF EXISTS notify_search_result();
DROP FUNCTION IF EXISTS auto_update_fts();
DROP TABLE IF EXISTS search_results;
