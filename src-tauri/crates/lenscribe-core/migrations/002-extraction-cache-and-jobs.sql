BEGIN;
CREATE INDEX IF NOT EXISTS files_image_hash ON files(image_hash, processor);
CREATE TABLE extraction_cache (image_hash TEXT NOT NULL, processor TEXT NOT NULL, text TEXT NOT NULL, PRIMARY KEY(image_hash, processor));
INSERT OR IGNORE INTO extraction_cache SELECT image_hash, processor, text FROM files WHERE processor IS NOT NULL AND text IS NOT NULL ORDER BY id;
CREATE TABLE extraction_jobs (id INTEGER PRIMARY KEY AUTOINCREMENT, file_id INTEGER NOT NULL UNIQUE REFERENCES files(id) ON DELETE CASCADE, image_hash TEXT NOT NULL, force INTEGER NOT NULL);
PRAGMA user_version = 2;
COMMIT;
