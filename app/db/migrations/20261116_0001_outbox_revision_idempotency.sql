BEGIN;
SET LOCAL lock_timeout = '5s';

-- Graph projection payloads reference a canonical SQL row and may be byte-identical
-- between two legitimate source revisions. The old payload-hash uniqueness prevents
-- emitting the later revision and leaves Neo4j stale.
-- Keep idx_sync_outbox_idempotency: same logical event/revision still deduplicates.
-- No historical events, leases, payloads or target states are altered.
DROP INDEX IF EXISTS system.idx_sync_outbox_dedup;

COMMIT;
