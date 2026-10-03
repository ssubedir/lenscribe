use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

pub(crate) struct ExtractionCacheRepository<'a> {
    connection: &'a Connection,
}

impl<'a> ExtractionCacheRepository<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn get(&self, image_hash: &str, processor: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT text FROM extraction_cache WHERE image_hash = ?1 AND processor = ?2",
                params![image_hash, processor],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(super) fn store(&self, image_hash: &str, processor: &str, text: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO extraction_cache(image_hash, processor, text) VALUES (?1, ?2, ?3)
             ON CONFLICT(image_hash, processor) DO UPDATE SET text = excluded.text",
            params![image_hash, processor, text],
        )?;
        Ok(())
    }
}
