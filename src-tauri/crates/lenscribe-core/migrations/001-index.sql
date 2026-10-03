BEGIN;
CREATE TABLE folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    root_hash TEXT NOT NULL
);
CREATE TABLE files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    image_hash TEXT NOT NULL,
    image_length INTEGER NOT NULL,
    text_hash TEXT,
    record_hash TEXT NOT NULL,
    processor TEXT,
    text TEXT,
    UNIQUE(folder_id, relative_path)
);
CREATE VIRTUAL TABLE files_fts USING fts5(relative_path, text, content='files', content_rowid='id');
CREATE TRIGGER files_insert AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, relative_path, text) VALUES(new.id, new.relative_path, new.text);
END;
CREATE TRIGGER files_delete AFTER DELETE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, text) VALUES('delete', old.id, old.relative_path, old.text);
END;
CREATE TRIGGER files_update AFTER UPDATE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, relative_path, text) VALUES('delete', old.id, old.relative_path, old.text);
    INSERT INTO files_fts(rowid, relative_path, text) VALUES(new.id, new.relative_path, new.text);
END;
PRAGMA user_version = 1;
COMMIT;
