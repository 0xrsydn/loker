-- Model answers for role classification (classify::RoleAnswer as JSON),
-- keyed by classify::answer_key: vocabulary, question and what the model
-- saw of the job. A cache: deleting it costs only new calls. Kept apart
-- from canonical_jobs so an unchanged job is never asked twice, even when
-- it is merged, split or rebuilt.
CREATE TABLE role_answers (
  key        TEXT PRIMARY KEY,
  model      TEXT NOT NULL,
  answer     TEXT NOT NULL,
  created_at TEXT NOT NULL
);
