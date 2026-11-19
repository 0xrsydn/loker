-- A canonical job's role (role::JobRole as JSON, NULL when nothing places
-- it) and the classifier stamp it was computed with. Derived from the
-- stored job: when the stamp differs from classify::STAMP, `Store::open`
-- classifies the job again, so a rule change needs no re-fetch.
ALTER TABLE canonical_jobs ADD COLUMN role TEXT;
ALTER TABLE canonical_jobs ADD COLUMN role_stamp INTEGER NOT NULL DEFAULT 0;
