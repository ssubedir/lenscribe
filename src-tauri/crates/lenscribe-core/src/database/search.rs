use rusqlite::{params, Connection};

use crate::{Error, Result, SearchHit};

use super::rows::{file_row, FILE_COLUMNS};

pub(crate) struct SearchRepository<'a> {
    connection: &'a Connection,
}

impl<'a> SearchRepository<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub fn query(
        &self,
        text: &str,
        folder_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        if text.len() > 4096 {
            return Err(Error::InvalidInput(
                "search query exceeds 4096 bytes".into(),
            ));
        }
        // Search terms are literal. FTS operators and quotes cannot alter the query.
        let terms = text
            .split_whitespace()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let mut query = self.connection.prepare(&format!(
            "SELECT {FILE_COLUMNS}, folders.path, snippet(files_fts, 1, '[', ']', ' … ', 24)
             FROM files_fts JOIN files f ON f.id = files_fts.rowid
             JOIN folders ON folders.id = f.folder_id
             WHERE files_fts MATCH ?1 AND (?2 IS NULL OR f.folder_id = ?2)
             ORDER BY bm25(files_fts), f.id LIMIT ?3",
        ))?;
        let hits = query
            .query_map(
                params![terms.join(" AND "), folder_id, limit.clamp(1, 100) as i64],
                |row| {
                    Ok(SearchHit {
                        file: file_row(row)?,
                        folder_path: row.get(8)?,
                        snippet: row.get(9)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(hits)
    }
}
