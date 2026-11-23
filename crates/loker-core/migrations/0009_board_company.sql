-- The company behind a board, as its ATS names it when the board is checked:
-- Greenhouse's company_name, the organization on an Ashby job page, a Lever
-- page's title. website is the company's own site, where Ashby gives one.
ALTER TABLE boards ADD COLUMN name TEXT;
ALTER TABLE boards ADD COLUMN website TEXT;
