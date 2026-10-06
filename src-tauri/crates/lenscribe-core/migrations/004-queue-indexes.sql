BEGIN;
ALTER TABLE extraction_jobs ADD COLUMN ready_at_ms INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS files_pending ON files(folder_id, id) WHERE processor IS NULL;
CREATE INDEX IF NOT EXISTS extraction_failures_recovery ON extraction_failures(recovery_id, retry_at_ms);
CREATE INDEX extraction_jobs_ready ON extraction_jobs(ready_at_ms, id);
PRAGMA user_version = 4;
COMMIT;
