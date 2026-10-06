//! V1 keeps the original bytes intact and the extracted text uncompressed.
//! Lengths and a payload checksum in a fixed-size ASCII footer make reads bounded.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

const START: &[u8] = b"\nLENSCRIBE-TEXT-V1\n";
const END: &str = "\nLENSCRIBE-END-V1 ";
const FOOTER_LEN: usize = END.len() + 20 + 1 + 20 + 1 + 64 + 1;
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = MAX_TEXT_BYTES + 32 * 1024;
const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextTrailer {
    pub image_hash: String,
    /// A stable provider/model/settings identifier; timestamps do not belong here.
    pub processor: String,
    pub text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    image_hash: String,
    processor: String,
}

#[derive(Clone, Debug)]
pub struct InspectedImage {
    pub image_hash: String,
    pub image_length: u64,
    pub mime_type: &'static str,
    pub trailer: Option<TextTrailer>,
}

pub fn supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
}

pub fn inspect(path: &Path) -> Result<InspectedImage> {
    let mut file = File::open(path)?;
    let (image_length, trailer) = read_trailer(&mut file)?;
    let mime_type = mime_type(&mut file, path, image_length)?;
    let image_hash = hash_prefix(&mut file, image_length)?;
    if trailer
        .as_ref()
        .is_some_and(|trailer| trailer.image_hash != image_hash)
    {
        return Err(Error::InvalidTrailer(
            "image checksum does not match".into(),
        ));
    }
    Ok(InspectedImage {
        image_hash,
        image_length,
        mime_type,
        trailer,
    })
}

/// Returns only the original image bytes, never the Lenscribe text trailer.
pub fn original_bytes(path: &Path, expected_hash: &str) -> Result<Vec<u8>> {
    let image = inspect(path)?;
    if image.image_hash != expected_hash {
        return Err(Error::ImageChanged);
    }
    if image.image_length > MAX_IMAGE_BYTES {
        return Err(Error::InvalidInput(
            "image exceeds the 128 MiB extraction limit".into(),
        ));
    }
    let mut file = File::open(path)?;
    let mut bytes = Vec::with_capacity(image.image_length as usize);
    Read::by_ref(&mut file)
        .take(image.image_length)
        .read_to_end(&mut bytes)?;
    if hash_bytes(&bytes) != expected_hash {
        return Err(Error::ImageChanged);
    }
    Ok(bytes)
}

/// Replaces only Lenscribe's own trailer using a temporary file in the same directory.
/// The expected hash prevents stale extraction responses from changing a newer image.
pub fn write_text(
    path: &Path,
    expected_hash: &str,
    text: &str,
    processor: &str,
) -> Result<InspectedImage> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(Error::InvalidInput("extracted text exceeds 16 MiB".into()));
    }
    if processor.trim().is_empty() || processor.len() > 4096 {
        return Err(Error::InvalidInput(
            "processor must contain 1–4096 bytes".into(),
        ));
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(Error::InvalidInput(
            "cannot write an image through a symbolic link".into(),
        ));
    }
    let before = inspect(path)?;
    if before.image_hash != expected_hash {
        return Err(Error::ImageChanged);
    }
    let trailer = TextTrailer {
        image_hash: before.image_hash.clone(),
        processor: processor.into(),
        text: text.into(),
    };
    if before.trailer.as_ref() == Some(&trailer) {
        return Ok(before);
    }
    let mut payload = serde_json::to_vec(&Header {
        image_hash: trailer.image_hash.clone(),
        processor: trailer.processor.clone(),
    })?;
    payload.push(b'\n');
    payload.extend_from_slice(text.as_bytes());
    let footer = format!(
        "{END}{:020} {:020} {}\n",
        before.image_length,
        payload.len(),
        hash_bytes(&payload)
    );
    debug_assert_eq!(footer.len(), FOOTER_LEN);

    let metadata = fs::metadata(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".lenscribe-")
        .tempfile_in(parent)?;
    let mut original = File::open(path)?;
    let copied = std::io::copy(
        &mut Read::by_ref(&mut original).take(before.image_length),
        &mut temporary,
    )?;
    if copied != before.image_length {
        return Err(Error::ImageChanged);
    }
    drop(original);
    if hash_prefix(temporary.as_file_mut(), before.image_length)? != expected_hash {
        return Err(Error::ImageChanged);
    }
    temporary.as_file_mut().seek(SeekFrom::End(0))?;
    temporary.write_all(START)?;
    temporary.write_all(&payload)?;
    temporary.write_all(footer.as_bytes())?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.as_file().sync_all()?;

    // Detect normal concurrent saves before replacing the file. The core also serializes its writers.
    let current = fs::metadata(path)?;
    if current.len() != metadata.len() || current.modified()? != metadata.modified()? {
        return Err(Error::ImageChanged);
    }
    temporary
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    Ok(InspectedImage {
        trailer: Some(trailer),
        ..before
    })
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn read_trailer(file: &mut File) -> Result<(u64, Option<TextTrailer>)> {
    let length = file.metadata()?.len();
    if length < FOOTER_LEN as u64 {
        return Ok((length, None));
    }
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
    let mut footer = vec![0; FOOTER_LEN];
    file.read_exact(&mut footer)?;
    if !footer.starts_with(END.as_bytes()) {
        // Do not reinterpret a partially truncated or displaced footer as original image bytes.
        let tail_length = length.min((FOOTER_LEN * 2) as u64);
        file.seek(SeekFrom::End(-(tail_length as i64)))?;
        let mut tail = vec![0; tail_length as usize];
        file.read_exact(&mut tail)?;
        if tail
            .windows(END.len())
            .any(|window| window == END.as_bytes())
        {
            return Err(Error::InvalidTrailer(
                "truncated or displaced footer".into(),
            ));
        }
        return Ok((length, None));
    }
    let invalid = |reason: &str| Error::InvalidTrailer(reason.into());
    let fields =
        std::str::from_utf8(&footer[END.len()..]).map_err(|_| invalid("footer is not ASCII"))?;
    if !fields.ends_with('\n') {
        return Err(invalid("footer has no final newline"));
    }
    let parts: Vec<_> = fields.trim_end_matches('\n').split(' ').collect();
    if parts.len() != 3 || parts[0].len() != 20 || parts[1].len() != 20 || parts[2].len() != 64 {
        return Err(invalid("invalid footer fields"));
    }
    let image_length: u64 = parts[0]
        .parse()
        .map_err(|_| invalid("invalid image length"))?;
    let payload_length: u64 = parts[1]
        .parse()
        .map_err(|_| invalid("invalid payload length"))?;
    if payload_length > MAX_PAYLOAD_BYTES as u64 {
        return Err(invalid("payload exceeds size limit"));
    }
    let expected_length = image_length
        .checked_add(START.len() as u64)
        .and_then(|length| length.checked_add(payload_length))
        .and_then(|length| length.checked_add(FOOTER_LEN as u64));
    if expected_length != Some(length) {
        return Err(invalid("lengths do not match the file"));
    }
    file.seek(SeekFrom::Start(image_length))?;
    let mut start = vec![0; START.len()];
    file.read_exact(&mut start)?;
    if start != START {
        return Err(invalid("missing start marker"));
    }
    let mut payload = vec![0; payload_length as usize];
    file.read_exact(&mut payload)?;
    if hash_bytes(&payload) != parts[2] {
        return Err(invalid("payload checksum does not match"));
    }
    let separator = payload
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| invalid("missing header separator"))?;
    let header: Header = serde_json::from_slice(&payload[..separator])
        .map_err(|_| invalid("invalid JSON header"))?;
    if header.processor.trim().is_empty() || header.processor.len() > 4096 {
        return Err(invalid("invalid processor identifier"));
    }
    let text = String::from_utf8(payload[separator + 1..].to_vec())
        .map_err(|_| invalid("text is not UTF-8"))?;
    if text.len() > MAX_TEXT_BYTES {
        return Err(invalid("text exceeds size limit"));
    }
    Ok((
        image_length,
        Some(TextTrailer {
            image_hash: header.image_hash,
            processor: header.processor,
            text,
        }),
    ))
}

fn mime_type(file: &mut File, path: &Path, image_length: u64) -> Result<&'static str> {
    file.seek(SeekFrom::Start(0))?;
    let mut signature = [0; 12];
    let length = Read::by_ref(file).take(image_length).read(&mut signature)?;
    if length >= 8 && signature[..8] == *b"\x89PNG\r\n\x1a\n" {
        Ok("image/png")
    } else if length >= 3 && signature[..3] == [0xff, 0xd8, 0xff] {
        Ok("image/jpeg")
    } else if length == 12 && signature[..4] == *b"RIFF" && signature[8..12] == *b"WEBP" {
        let riff_size = u32::from_le_bytes(signature[4..8].try_into().unwrap());
        // The declared container must fit within the original bytes, never the text trailer.
        // Unmarked trailing bytes remain part of image identity, just as for PNG and JPEG.
        if riff_size < 12 || riff_size % 2 != 0 || u64::from(riff_size) + 8 > image_length {
            return Err(Error::UnsupportedImage(path.into()));
        }
        Ok("image/webp")
    } else {
        Err(Error::UnsupportedImage(path.into()))
    }
}

fn hash_prefix(file: &mut File, length: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = file.take(length);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut read_length = 0;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        read_length += count as u64;
    }
    if read_length != length {
        return Err(Error::ImageChanged);
    }
    Ok(hex::encode(hasher.finalize()))
}
