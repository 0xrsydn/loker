-- no-transaction
-- Cross-source dedup.
--
-- A canonical job is now the merge of every source job linked to it:
--   * source_jobs keeps each source's normalized job, its own content hash
--     (idempotency per source) and its match keys (company_key, title_key);
--   * canonical_url is no longer UNIQUE: it changes when a better source joins
--     (an employer board beats an aggregator). Identity is the stable `id`
--     plus the source_jobs links.
--
-- Rebuilding canonical_jobs (to drop UNIQUE) follows SQLite's documented
-- procedure: foreign keys off outside the transaction, rebuild, check, commit.
-- A deferred FK check can't be used: DROP TABLE counts every child row as a
-- violation and re-creating the parent never uncounts it.
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE canonical_jobs_new (
  id                TEXT PRIMARY KEY,         -- "j_" + 10 hex; assigned once, never recomputed
  canonical_url     TEXT NOT NULL,
  apply_url         TEXT,
  title             TEXT NOT NULL,
  company           TEXT NOT NULL,
  seniority         TEXT,
  employment_type   TEXT,
  eligibility       TEXT NOT NULL,            -- JSON (model::Eligibility)
  remote_scope      TEXT GENERATED ALWAYS AS (json_extract(eligibility, '$.scope')) STORED,
  pay               TEXT,                     -- JSON (model::Pay)
  pay_usd_month_max INTEGER GENERATED ALWAYS AS (json_extract(pay, '$.usdMonthMax')) STORED,
  tech_tags         TEXT NOT NULL DEFAULT '[]',
  description_md    TEXT,
  content_hash      TEXT NOT NULL,            -- hash of the merged fields
  posted_at         TEXT,
  first_seen        TEXT NOT NULL,
  last_seen         TEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT 'active'
);
INSERT INTO canonical_jobs_new (id, canonical_url, apply_url, title, company, seniority, employment_type,
    eligibility, pay, tech_tags, description_md, content_hash, posted_at, first_seen, last_seen, status)
  SELECT id, canonical_url, apply_url, title, company, seniority, employment_type,
    eligibility, pay, tech_tags, description_md, content_hash, posted_at, first_seen, last_seen, status
  FROM canonical_jobs;
DROP TABLE canonical_jobs;
ALTER TABLE canonical_jobs_new RENAME TO canonical_jobs;
CREATE INDEX canonical_jobs_url ON canonical_jobs (canonical_url);
CREATE INDEX canonical_jobs_status_posted ON canonical_jobs (status, posted_at);
CREATE INDEX canonical_jobs_first_seen ON canonical_jobs (first_seen);

ALTER TABLE source_jobs ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'aggregator';  -- 'employer' | 'aggregator'
ALTER TABLE source_jobs ADD COLUMN job TEXT;                     -- normalized NewJob JSON, NULL for pre-0002 rows
ALTER TABLE source_jobs ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE source_jobs ADD COLUMN company_key TEXT NOT NULL DEFAULT '';
ALTER TABLE source_jobs ADD COLUMN title_key TEXT NOT NULL DEFAULT '';
ALTER TABLE source_jobs ADD COLUMN first_seen TEXT;
CREATE INDEX source_jobs_canonical ON source_jobs (canonical_job_id);
CREATE INDEX source_jobs_key ON source_jobs (company_key, title_key);

ALTER TABLE ingestion_runs ADD COLUMN jobs_merged INTEGER;
ALTER TABLE ingestion_runs ADD COLUMN jobs_ambiguous INTEGER;

-- Abort (and roll back) if the rebuild broke any source_jobs → canonical_jobs link.
CREATE TEMP TABLE _fk_check (violations INTEGER CHECK (violations = 0));
INSERT INTO _fk_check SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE _fk_check;
COMMIT;
PRAGMA foreign_keys = ON;
