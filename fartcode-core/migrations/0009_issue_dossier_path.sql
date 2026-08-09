-- Feature dossiers (E19-01, #70; ADR-0038 item 1): the repo-relative path
-- of the card's `docs/features/<slug>.md`, written when the dossier is born
-- with the worktree at the card's first `agent_step` entry.
--
-- Nullable by design and NULL on every existing row: a card with no dossier
-- (never dispatched, consent declined, or a write that failed) is the normal
-- case, not a defect. Nothing derives behavior from it being set.
ALTER TABLE issues ADD COLUMN dossier_path TEXT;
