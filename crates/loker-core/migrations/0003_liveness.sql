-- Liveness: listings close, and a job is active while at least one of its
-- listings is live.
--
-- A listing goes `gone` when
--   * a complete fetch of its source (a company's whole board) no longer has it  -> 'absent'
--   * the source's own expiry date has passed                                      -> 'expired'
--   * nobody has seen it for 30 days (backstop for partial feeds)                  -> 'stale'
-- and comes back `live` if it is seen again.
ALTER TABLE source_jobs ADD COLUMN status TEXT NOT NULL DEFAULT 'live';  -- 'live' | 'gone'
ALTER TABLE source_jobs ADD COLUMN gone_at TEXT;
ALTER TABLE source_jobs ADD COLUMN gone_reason TEXT;                      -- 'absent' | 'expired' | 'stale'
ALTER TABLE source_jobs ADD COLUMN expires_at TEXT;                       -- from the source, when it says
CREATE INDEX source_jobs_live ON source_jobs (status, source_id);

ALTER TABLE canonical_jobs ADD COLUMN closed_at TEXT;                     -- status = 'closed' since

ALTER TABLE ingestion_runs ADD COLUMN jobs_closed INTEGER;
ALTER TABLE ingestion_runs ADD COLUMN jobs_reopened INTEGER;
ALTER TABLE ingestion_runs ADD COLUMN jobs_held INTEGER;
