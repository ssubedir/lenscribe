BEGIN;
CREATE TABLE extraction_failures (
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    recovery_id TEXT NOT NULL,
    image_hash TEXT NOT NULL,
    request_id INTEGER,
    error TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    retry_at_ms INTEGER,
    PRIMARY KEY(file_id, recovery_id)
);
CREATE TABLE extraction_endpoints (
    recovery_id TEXT PRIMARY KEY,
    retry_at_ms INTEGER,
    next_request_ms INTEGER NOT NULL DEFAULT 0,
    blocked_error TEXT
);
PRAGMA user_version = 3;
COMMIT;
