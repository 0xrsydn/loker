-- Stem words in the search index (porter), so "engineer" also matches
-- "engineering" and "developers" matches "developer". Measured in
-- eval/search: `plain or` -> `porter or` lifts nDCG@10 from 0.57 to 0.64.
-- The index holds only derived text, so it is rebuilt from canonical_jobs.
DROP TABLE jobs_fts;

CREATE VIRTUAL TABLE jobs_fts USING fts5 (
  job_id UNINDEXED, title, company, description,
  tokenize = 'porter unicode61 remove_diacritics 2'
);

INSERT INTO jobs_fts (job_id, title, company, description)
SELECT id, title, company, description_md FROM canonical_jobs;
