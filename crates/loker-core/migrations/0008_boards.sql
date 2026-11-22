-- Company boards found by `loker discover` (board::Board), one row per
-- board key: Ashby and Greenhouse slugs compare without case, Lever's with.
-- slug is the spelling to fetch, set from the ATS's own answer when checked.
-- status: unchecked (found, not yet asked), live (has jobs), empty (exists,
-- no jobs), missing (the ATS says no such board).
CREATE TABLE boards (
  key         TEXT PRIMARY KEY,
  ats         TEXT NOT NULL,
  slug        TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'unchecked',
  jobs        INTEGER,
  first_found TEXT NOT NULL,
  checked_at  TEXT,
  error       TEXT
);

CREATE INDEX boards_status ON boards (status, ats);

-- Where each board was found: 'commoncrawl:CC-MAIN-2026-43', 'file:fd.txt'.
CREATE TABLE board_finds (
  key      TEXT NOT NULL REFERENCES boards (key),
  finder   TEXT NOT NULL,
  found_at TEXT NOT NULL,
  PRIMARY KEY (key, finder)
);

-- A finder read to its end, so a crawl is read once.
CREATE TABLE discovery_runs (
  finder      TEXT PRIMARY KEY,
  finished_at TEXT NOT NULL,
  found       INTEGER NOT NULL
);
