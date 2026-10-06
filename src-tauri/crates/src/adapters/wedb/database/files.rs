use super::Database;
use crate::{Error, FileDetails, FilePage, FileRecord, Result};
pub(crate) struct FileRepository<'a> {
    database: &'a Database,
}
impl<'a> FileRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    pub fn get(&self, id: i64) -> Result<FileDetails> {
        let file = self
            .database
            .state
            .borrow()
            .files
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("file {id}")))?;
        let text = file
            .text_hash
            .as_ref()
            .map(|hash| self.database.store.get::<String>(&format!("texts/{hash}")))
            .transpose()?
            .flatten();
        if file.text_hash.is_some() && text.is_none() {
            return Err(Error::Storage("extracted text body is missing".into()));
        }
        Ok(FileDetails { file, text })
    }
    pub fn by_path(&self, folder: i64, path: &str) -> Result<FileDetails> {
        let id = self
            .database
            .state
            .borrow()
            .by_path(folder, path)
            .map(|file| file.id)
            .ok_or_else(|| Error::NotFound(path.into()))?;
        self.get(id)
    }
    pub(super) fn in_folder(&self, folder: i64) -> Result<Vec<FileRecord>> {
        let state = self.database.state.borrow();
        Ok(state
            .paths
            .range((folder, String::new())..)
            .take_while(|((id, _), _)| *id == folder)
            .filter_map(|(_, id)| state.files.get(id).cloned())
            .collect())
    }
    pub fn list(&self, folder: i64, query: &str, offset: usize, fuzzy: bool) -> Result<FilePage> {
        self.database.folders().get(folder)?;
        if query.len() > 1024 {
            return Err(Error::InvalidInput("file search exceeds 1024 bytes".into()));
        }
        let page = self
            .database
            .search()
            .page(query, Some(folder), offset, 50, fuzzy)?;
        Ok(FilePage {
            files: page.hits.into_iter().map(|hit| hit.file).collect(),
            total: page.total,
            notice: page.notice,
        })
    }
}
