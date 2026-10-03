use rusqlite::{params, Connection, OptionalExtension};

use crate::{merkle::MerkleTree, Error, FolderProgress, FolderRecord, FolderSnapshot, Result};

use super::{files::FileRepository, rows::folder_row};

pub(crate) struct FolderRepository<'a> {
    connection: &'a Connection,
}

impl<'a> FolderRepository<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn ensure(&self, path: &str) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO folders(path, root_hash) VALUES (?1, ?2) ON CONFLICT(path) DO NOTHING",
            params![path, MerkleTree::new().root_hash()],
        )?;
        Ok(self
            .connection
            .query_row("SELECT id FROM folders WHERE path = ?1", [path], |row| {
                row.get(0)
            })?)
    }

    pub fn get(&self, id: i64) -> Result<FolderRecord> {
        self.connection
            .query_row(
                "SELECT id, path, root_hash,
                    (SELECT count(*) FROM files WHERE folder_id = folders.id)
                 FROM folders WHERE id = ?1",
                [id],
                folder_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("folder {id}")))
    }

    pub fn list(&self) -> Result<Vec<FolderRecord>> {
        let mut query = self.connection.prepare(
            "SELECT id, path, root_hash,
                (SELECT count(*) FROM files WHERE folder_id = folders.id)
             FROM folders ORDER BY path",
        )?;
        let folders = query
            .query_map([], folder_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(folders)
    }

    pub fn snapshot(&self, folder_id: i64) -> Result<FolderSnapshot> {
        Ok(FolderSnapshot {
            folder: self.get(folder_id)?,
            files: FileRepository::new(self.connection).in_folder(folder_id)?,
        })
    }

    pub fn progress(&self) -> Result<Vec<FolderProgress>> {
        let mut query = self.connection.prepare(
            "SELECT folders.id, folders.path, folders.root_hash, count(f.id),
                count(CASE WHEN f.processor IS NULL OR EXISTS(
                    SELECT 1 FROM extraction_jobs j
                    WHERE j.file_id = f.id AND j.image_hash = f.image_hash
                ) THEN f.id END)
             FROM folders LEFT JOIN files f ON f.folder_id = folders.id
             GROUP BY folders.id ORDER BY folders.path",
        )?;
        let progress = query
            .query_map([], |row| {
                Ok(FolderProgress {
                    folder: folder_row(row)?,
                    pending_images: row.get::<_, i64>(4)? as usize,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(progress)
    }

    pub(super) fn update_root(&self, folder_id: i64, root_hash: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE folders SET root_hash = ?1 WHERE id = ?2 AND root_hash != ?1",
            params![root_hash, folder_id],
        )?;
        Ok(())
    }
}
