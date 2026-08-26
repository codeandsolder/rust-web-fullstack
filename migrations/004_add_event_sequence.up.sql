-- LISTEN/NOTIFY is not durable, so reconnect recovery needs a durable cursor.
--
-- A plain BIGSERIAL is not sufficient: sequence values are allocated before a
-- transaction commits, so two concurrent inserts can become visible/notify in
-- the opposite order. A consumer that advances a high-water mark would then
-- skip the late-committing lower value.
--
-- We serialize event-sequence allocation with a transaction-scoped advisory
-- lock. The lock is held until COMMIT/ROLLBACK, so every visible event_seq is
-- strictly increasing in commit order. This intentionally serializes writes to
-- search_results; for this demo/event-feed workload correctness is preferable
-- to maximizing insert throughput.

CREATE SEQUENCE search_results_event_seq_seq AS BIGINT START WITH 1;

ALTER TABLE search_results
    ADD COLUMN event_seq BIGINT;

-- Give pre-existing rows a deterministic history before enabling the trigger.
WITH ordered AS (
    SELECT id, row_number() OVER (ORDER BY created_at ASC, id ASC) AS seq
    FROM search_results
)
UPDATE search_results AS target
SET event_seq = ordered.seq
FROM ordered
WHERE target.id = ordered.id;

-- Position the sequence after the backfilled range. With is_called=false the
-- next nextval() returns exactly max+1.
SELECT setval(
    'search_results_event_seq_seq',
    COALESCE((SELECT MAX(event_seq) FROM search_results), 0) + 1,
    false
);

CREATE OR REPLACE FUNCTION assign_search_result_event_seq()
RETURNS TRIGGER AS $$
BEGIN
    -- Stable arbitrary 64-bit key reserved for this sequence allocator.
    PERFORM pg_advisory_xact_lock(734923847219374001);
    NEW.event_seq := nextval('search_results_event_seq_seq');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_search_result_event_seq
BEFORE INSERT ON search_results
FOR EACH ROW
EXECUTE FUNCTION assign_search_result_event_seq();

ALTER TABLE search_results
    ALTER COLUMN event_seq SET NOT NULL;

CREATE UNIQUE INDEX search_results_event_seq_idx
    ON search_results (event_seq);
