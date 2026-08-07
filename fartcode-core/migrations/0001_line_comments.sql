-- Line comment features (E4-10, #50): ARCHITECTURE.md §14 Phase-1 additions
-- on top of the base `line_comments` table from 0000, plus the bidirectional
-- task link (`tasks.source_comment_id` ↔ `line_comments.linked_task_id`).
ALTER TABLE line_comments ADD COLUMN source_side TEXT DEFAULT 'after';
--> statement-breakpoint
ALTER TABLE line_comments ADD COLUMN line_end INTEGER;
--> statement-breakpoint
ALTER TABLE line_comments ADD COLUMN linked_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL;
--> statement-breakpoint
ALTER TABLE line_comments ADD COLUMN resolved INTEGER DEFAULT 0;
--> statement-breakpoint
ALTER TABLE line_comments ADD COLUMN resolved_at TEXT;
--> statement-breakpoint
ALTER TABLE line_comments ADD COLUMN created_by TEXT DEFAULT 'user';
--> statement-breakpoint
ALTER TABLE tasks ADD COLUMN source_comment_id TEXT REFERENCES line_comments(id) ON DELETE SET NULL;
