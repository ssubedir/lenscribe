use rusqlite::{params, Connection, OptionalExtension};

use crate::{Error, FileDetails, FilePage, FileRecord, Result};

use super::{
    folders::FolderRepository,
    rows::{file_row, FILE_COLUMNS},
};

pub(crate) struct FileRepository<'a> {
    connection: &'a Connection,
}

impl<'a> FileRepository<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn get(&self, id: i64) -> Result<FileDetails> {
        self.connection
            .query_row(
                &format!("SELECT {FILE_COLUMNS}, f.text FROM files f WHERE id = ?1"),
                [id],
                |row| {
                    Ok(FileDetails {
                        file: file_row(row)?,
                        text: row.get(8)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("file {id}")))
    }

    pub fn by_path(&self, folder_id: i64, relative_path: &str) -> Result<FileDetails> {
        let id: i64 = self
            .connection
            .query_row(
                "SELECT id FROM files WHERE folder_id = ?1 AND relative_path = ?2",
                params![folder_id, relative_path],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(relative_path.into()))?;
        self.get(id)
    }

    pub(super) fn find_record(&self, id: i64) -> Result<Option<FileRecord>> {
        Ok(self
            .connection
            .query_row(
                &format!("SELECT {FILE_COLUMNS} FROM files f WHERE f.id = ?1"),
                [id],
                file_row,
            )
            .optional()?)
    }

    pub(super) fn in_folder(&self, folder_id: i64) -> Result<Vec<FileRecord>> {
        let mut query = self.connection.prepare(&format!(
            "SELECT {FILE_COLUMNS} FROM files f WHERE folder_id = ?1 ORDER BY relative_path",
        ))?;
        let files = query
            .query_map([folder_id], file_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(files)
    }

    pub fn list(&self, folder_id: i64, query: &str, offset: usize) -> Result<FilePage> {
        FolderRepository::new(self.connection).get(folder_id)?;
        if query.len() > 1024 {
            return Err(Error::InvalidInput(
                "filename filter exceeds 1024 bytes".into(),
            ));
        }
        let total: i64 = self.connection.query_row(
            "SELECT count(*) FROM files
             WHERE folder_id = ?1 AND instr(lower(relative_path), lower(?2)) > 0",
            params![folder_id, query],
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {FILE_COLUMNS} FROM files f
             WHERE f.folder_id = ?1 AND instr(lower(f.relative_path), lower(?2)) > 0
             ORDER BY f.relative_path LIMIT 50 OFFSET ?3",
        ))?;
        let files = statement
            .query_map(
                params![folder_id, query, i64::try_from(offset).unwrap_or(i64::MAX)],
                file_row,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(FilePage {
            files,
            total: total as usize,
        })
    }

    pub(super) fn upsert(&self, folder_id: i64, details: &FileDetails) -> Result<()> {
        let file = &details.file;
        let length = i64::try_from(file.image_length).map_err(|_| {
            Error::InvalidInput("image length exceeds SQLite's integer range".into())
        })?;
        self.connection.execute(
            "INSERT INTO files(folder_id, relative_path, image_hash, image_length,
                text_hash, record_hash, processor, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(folder_id, relative_path) DO UPDATE SET
                image_hash = excluded.image_hash, image_length = excluded.image_length,
                text_hash = excluded.text_hash, record_hash = excluded.record_hash,
                processor = excluded.processor, text = excluded.text
             WHERE files.record_hash != excluded.record_hash",
            params![
                folder_id,
                file.relative_path,
                file.image_hash,
                length,
                file.text_hash,
                file.record_hash,
                file.processor,
                details.text
            ],
        )?;
        Ok(())
    }

    pub(super) fn remove(&self, folder_id: i64, relative_path: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM files WHERE folder_id = ?1 AND relative_path = ?2",
            params![folder_id, relative_path],
        )?;
        Ok(())
    }
}
