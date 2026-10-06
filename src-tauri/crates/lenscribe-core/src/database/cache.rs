use super::{store::Change, Database};
use crate::{trailer::hash_bytes, Result};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
pub(super) struct CacheEntry {
    pub image_hash: String,
    pub processor: String,
    pub text_hash: String,
}
pub(super) fn cache_key(image: &str, processor: &str) -> String {
    format!("cache/{image}/{}", hash_bytes(processor.as_bytes()))
}
pub(super) fn store_changes(image: &str, processor: &str, text: &str) -> Result<Vec<Change>> {
    let text_hash = hash_bytes(text.as_bytes());
    Ok(vec![
        Change::put(format!("texts/{text_hash}"), &text)?,
        Change::put(
            cache_key(image, processor),
            &CacheEntry {
                image_hash: image.into(),
                processor: processor.into(),
                text_hash,
            },
        )?,
    ])
}
pub(crate) struct ExtractionCacheRepository<'a> {
    database: &'a Database,
}
impl<'a> ExtractionCacheRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    pub fn get(&self, image: &str, processor: &str) -> Result<Option<String>> {
        let Some(entry) = self
            .database
            .store
            .get::<CacheEntry>(&cache_key(image, processor))?
        else {
            return Ok(None);
        };
        let text = self
            .database
            .store
            .get(&format!("texts/{}", entry.text_hash))?
            .ok_or_else(|| crate::Error::Storage("missing cached text body".into()))?;
        Ok(Some(text))
    }
}
