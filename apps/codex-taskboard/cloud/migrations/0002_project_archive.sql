ALTER TABLE projects ADD COLUMN archived_at TEXT;
CREATE INDEX projects_archived_created ON projects(archived_at, created_at, id);
