-- LISTEN/NOTIFY is not durable, so reconnect recovery needs a monotonic cursor.
-- UUIDv4 ordering is random and cannot safely identify rows inserted after a
-- disconnect. BIGSERIAL gives every search-result row a durable insertion
-- sequence that can be replayed and paged without gaps.
ALTER TABLE search_results
    ADD COLUMN event_seq BIGSERIAL NOT NULL;

CREATE UNIQUE INDEX search_results_event_seq_idx
    ON search_results (event_seq);
