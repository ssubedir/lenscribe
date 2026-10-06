use super::{
    store::{id_key, Change},
    Database,
};
use crate::{merkle::MerkleTree, FolderProgress, FolderRecord, FolderSnapshot, Result};
pub(crate) struct FolderRepository<'a> {
    database: &'a Database,
}
impl<'a> FolderRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    pub fn ensure(&self, path: &str) -> Result<i64> {
        if let Some(folder) = self
            .database
            .state
            .borrow()
            .folders
            .values()
            .find(|folder| folder.path == path)
        {
            return Ok(folder.id);
        }
        let id = self
            .database
            .store
            .get::<i64>("seq/folder")?
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| crate::Error::Storage("folder IDs exhausted".into()))?;
        let folder = FolderRecord {
            id,
            path: path.into(),
            root_hash: MerkleTree::new().root_hash(),
            image_count: 0,
        };
        self.database.commit(vec![
            Change::put(id_key("folders", id), &folder)?,
            Change::put("seq/folder", &id)?,
        ])?;
        Ok(id)
    }
    pub fn get(&self, id: i64) -> Result<FolderRecord> {
        let state = self.database.state.borrow();
        let mut folder = state.folder(id)?.clone();
        folder.image_count = state
            .paths
            .range((id, String::new())..)
            .take_while(|((folder, _), _)| *folder == id)
            .count();
        Ok(folder)
    }
    pub fn list(&self) -> Result<Vec<FolderRecord>> {
        let ids = self
            .database
            .state
            .borrow()
            .folders
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let mut folders = ids
            .into_iter()
            .map(|id| self.get(id))
            .collect::<Result<Vec<_>>>()?;
        folders.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(folders)
    }
    pub fn snapshot(&self, id: i64) -> Result<FolderSnapshot> {
        Ok(FolderSnapshot {
            folder: self.get(id)?,
            files: self.database.files().in_folder(id)?,
        })
    }
    pub fn progress(&self) -> Result<Vec<FolderProgress>> {
        let folders = self.list()?;
        let state = self.database.state.borrow();
        Ok(folders
            .into_iter()
            .map(|folder| FolderProgress {
                pending_images: state
                    .paths
                    .range((folder.id, String::new())..)
                    .take_while(|((id, _), _)| *id == folder.id)
                    .filter(|(_, id)| {
                        state.files.get(id).is_some_and(|file| {
                            file.processor.is_none()
                                || state
                                    .jobs
                                    .get(id)
                                    .is_some_and(|job| job.image_hash == file.image_hash)
                        })
                    })
                    .count(),
                folder,
            })
            .collect())
    }
}
