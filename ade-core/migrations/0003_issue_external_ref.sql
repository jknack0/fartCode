-- GitHub-issue import marker (E17 dogfood, #54 follow-up): imported GitHub
-- issues carry their source URL so the board stays linked to the tracker.
ALTER TABLE issues ADD COLUMN external_ref TEXT;
