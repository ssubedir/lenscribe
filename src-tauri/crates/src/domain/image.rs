use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextTrailer {
    pub image_hash: String,
    /// A stable provider/model/settings identifier; timestamps do not belong here.
    pub processor: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct InspectedImage {
    pub image_hash: String,
    pub image_length: u64,
    pub mime_type: &'static str,
    pub trailer: Option<TextTrailer>,
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn path_string(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::InvalidInput("paths must be valid Unicode".into()))
}
