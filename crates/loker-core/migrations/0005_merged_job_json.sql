-- no-transaction
-- A canonical job is stored as its merged job JSON (model::NewJob), hashed as
-- a whole, with generated columns only for what SQL filters, sorts or shows.
-- A new job field then needs no migration and no edit to a hand-listed hash.
--
-- The JSON is first assembled here from the old columns, with an empty
-- content_hash; the first `Store::open` after this migration rebuilds every
-- such job from its listings (rule 1), which writes the exact merged JSON.
--
-- Same rebuild procedure as 0002: foreign keys off outside the transaction,
-- rebuild, check, commit.
PRAGMA foreign_keys = OFF;
BEGIN;

CREATE TABLE canonical_jobs_new (
  id            TEXT PRIMARY KEY,          -- "j_" + 10 hex; assigned once, never recomputed
  job           TEXT NOT NULL,             -- merged job JSON (model::NewJob)
  content_hash  TEXT NOT NULL,             -- hash of status + job; '' = rebuild from listings
  first_seen    TEXT NOT NULL,
  last_seen     TEXT NOT NULL,
  status        TEXT NOT NULL DEFAULT 'active',
  closed_at     TEXT,
  canonical_url TEXT GENERATED ALWAYS AS (json_extract(job, '$.canonicalUrl')) STORED,
  remote_scope  TEXT GENERATED ALWAYS AS (json_extract(job, '$.eligibility.scope')) STORED,
  posted_at     TEXT GENERATED ALWAYS AS (json_extract(job, '$.postedAt')) STORED,
  title         TEXT GENERATED ALWAYS AS (json_extract(job, '$.title')) VIRTUAL,
  company       TEXT GENERATED ALWAYS AS (json_extract(job, '$.company')) VIRTUAL
);
INSERT INTO canonical_jobs_new (id, job, content_hash, first_seen, last_seen, status, closed_at)
  SELECT id,
    json_object(
      'sourceId', '', 'sourceJobId', '', 'sourceUrl', canonical_url,
      'canonicalUrl', canonical_url, 'applyUrl', apply_url,
      'title', title, 'company', company,
      'seniority', seniority, 'employmentType', employment_type,
      'eligibility', json(eligibility), 'pay', json(pay), 'techTags', json(tech_tags),
      'descriptionMd', description_md, 'postedAt', posted_at),
    '', first_seen, last_seen, status, closed_at
  FROM canonical_jobs;
DROP TABLE canonical_jobs;
ALTER TABLE canonical_jobs_new RENAME TO canonical_jobs;
CREATE INDEX canonical_jobs_url ON canonical_jobs (canonical_url);
CREATE INDEX canonical_jobs_status_posted ON canonical_jobs (status, posted_at);
CREATE INDEX canonical_jobs_first_seen ON canonical_jobs (first_seen);

-- Abort (and roll back) if the rebuild broke any source_jobs → canonical_jobs link.
CREATE TEMP TABLE _fk_check (violations INTEGER CHECK (violations = 0));
INSERT INTO _fk_check SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE _fk_check;
COMMIT;
PRAGMA foreign_keys = ON;
