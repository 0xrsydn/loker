-- What search filters on, stored with each canonical job instead of being read
-- from its JSON and description on every search. derived_stamp names the rules
-- that produced seniority, visa and experience (derived::STAMP); a row with a
-- different stamp is derived again when the store opens. scope and posted copy
-- remote_scope and posted_at: SQLite can't answer a query from an index that
-- holds a generated column, and canonical_jobs_search must cover every filter
-- so a search never reads the job JSON of a row it doesn't return; _search is
-- for a full-text match, _listing for a search without words. fts_rowid is the
-- job's row in jobs_fts: joining on it reads no full-text content, and a
-- rebuild deletes the old row by it instead of scanning for job_id.
ALTER TABLE canonical_jobs ADD COLUMN eligibility TEXT;
ALTER TABLE canonical_jobs ADD COLUMN scope TEXT;
ALTER TABLE canonical_jobs ADD COLUMN posted TEXT;
ALTER TABLE canonical_jobs ADD COLUMN seniority TEXT;
ALTER TABLE canonical_jobs ADD COLUMN visa TEXT;
ALTER TABLE canonical_jobs ADD COLUMN experience TEXT;
ALTER TABLE canonical_jobs ADD COLUMN min_years INTEGER;
ALTER TABLE canonical_jobs ADD COLUMN derived_stamp INTEGER NOT NULL DEFAULT 0;
ALTER TABLE canonical_jobs ADD COLUMN fts_rowid INTEGER;
UPDATE canonical_jobs SET fts_rowid = f.rid
  FROM (SELECT job_id, max(rowid) AS rid FROM jobs_fts GROUP BY job_id) AS f
  WHERE f.job_id = canonical_jobs.id;
CREATE INDEX canonical_jobs_search
  ON canonical_jobs (fts_rowid, status, posted, scope, seniority, min_years, role, eligibility, id);
CREATE INDEX canonical_jobs_listing
  ON canonical_jobs (status, posted, id, scope, seniority, min_years, role, eligibility);
CREATE INDEX canonical_jobs_derived_stamp ON canonical_jobs (derived_stamp);
CREATE INDEX canonical_jobs_role_stamp ON canonical_jobs (role_stamp);
CREATE INDEX canonical_jobs_unbuilt ON canonical_jobs (id) WHERE content_hash = '';
