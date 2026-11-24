-- Every ingest batch looks for live listings past their expiry or not fetched
-- for STALE_AFTER_DAYS. Without these the check reads every live row, raw JSON
-- included, once per board: O(listings x boards). They hold no status column,
-- so the planner never prefers them over source_jobs_key or source_jobs_live
-- for another query; Store::ingest names them with INDEXED BY.
CREATE INDEX source_jobs_stale ON source_jobs (julianday(fetched_at)) WHERE status = 'live';
CREATE INDEX source_jobs_expiry ON source_jobs (julianday(expires_at))
  WHERE status = 'live' AND expires_at IS NOT NULL;
