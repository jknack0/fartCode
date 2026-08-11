-- 0012: project path uniqueness is per connection (E12-04).
--
-- `idx_projects_path` made the path globally unique, which is right for local
-- projects and wrong for remote ones: `/srv/repos/app` on two different hosts
-- is two different repositories. The replacement keys on the path AND the
-- connection, with `COALESCE(..., '')` so local rows (NULL connection) still
-- collide with each other — a bare `(path, ssh_connection_id)` index would
-- treat every NULL as distinct and silently allow duplicate local projects.
DROP INDEX IF EXISTS idx_projects_path;--> statement-breakpoint
CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_path_connection
    ON projects(path, COALESCE(ssh_connection_id, ''));
