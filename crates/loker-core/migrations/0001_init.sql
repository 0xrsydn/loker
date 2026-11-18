CREATE TABLE canonical_jobs (
  id                TEXT PRIMARY KEY,         -- "j_" + 10 hex of blake3(canonical_url)
  canonical_url     TEXT NOT NULL UNIQUE,
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
  content_hash      TEXT NOT NULL,            -- unchanged re-ingests are no-ops
  posted_at         TEXT,
  first_seen        TEXT NOT NULL,
  last_seen         TEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT 'active'
);
CREATE INDEX canonical_jobs_status_posted ON canonical_jobs (status, posted_at);
CREATE INDEX canonical_jobs_first_seen ON canonical_jobs (first_seen);

CREATE TABLE source_jobs (
  source_id        TEXT NOT NULL,
  source_job_id    TEXT NOT NULL,
  source_url       TEXT NOT NULL,
  canonical_job_id TEXT NOT NULL REFERENCES canonical_jobs (id),
  raw_data         TEXT NOT NULL,
  fetched_at       TEXT NOT NULL,
  PRIMARY KEY (source_id, source_job_id)
);

CREATE TABLE ingestion_runs (
  id             INTEGER PRIMARY KEY,
  source_id      TEXT NOT NULL,
  started_at     TEXT NOT NULL,
  completed_at   TEXT,
  status         TEXT NOT NULL,                -- 'completed' | 'failed'
  error          TEXT,
  jobs_fetched   INTEGER, jobs_new INTEGER, jobs_updated INTEGER, jobs_unchanged INTEGER, jobs_skipped INTEGER
);

CREATE VIRTUAL TABLE jobs_fts USING fts5 (
  job_id UNINDEXED, title, company, description,
  tokenize = 'unicode61 remove_diacritics 2'
);
