use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::error::{ConvertError, Result};

pub const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_unpacked_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_entries: usize,
    pub memory_budget_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * MIB,
            max_unpacked_bytes: 256 * MIB,
            max_entry_bytes: 64 * MIB,
            max_entries: 4096,
            memory_budget_bytes: 512 * MIB,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub path: PathBuf,
    pub input_sha256: String,
    entries: BTreeMap<String, Vec<u8>>,
    pub unpacked_bytes: u64,
}

/// Estimate the peak per-document memory use without materializing ZIP
/// entries. Batch conversion uses this before starting a worker so the sum of
/// concurrently active estimates stays within the shared budget.
pub fn estimate_memory_for_path(path: impl AsRef<Path>, limits: &Limits) -> Result<u64> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(ConvertError::MissingInput(path.to_path_buf()));
    }
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > limits.max_input_bytes {
        return Err(ConvertError::InputTooLarge {
            limit: limits.max_input_bytes / MIB,
        });
    }
    let mut input = std::fs::File::open(path)?;
    let mut signature = [0_u8; 4];
    input.read_exact(&mut signature)?;
    if signature != [0x50, 0x4b, 0x03, 0x04] {
        return Err(ConvertError::UnsupportedFormat(path.to_path_buf()));
    }
    input.seek(SeekFrom::Start(0))?;
    let mut archive = ZipArchive::new(input)?;
    if archive.len() > limits.max_entries {
        return Err(ConvertError::TooManyEntries {
            limit: limits.max_entries,
        });
    }
    let mut normalized_names = BTreeSet::new();
    let mut unpacked_bytes = 0_u64;
    let mut image_bytes = 0_u64;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let raw_name = file.name().to_owned();
        let name = normalize_entry_name(&raw_name)?;
        if !normalized_names.insert(name.clone()) {
            return Err(ConvertError::DuplicateZipPath(name));
        }
        if file.encrypted() {
            return Err(ConvertError::EncryptedOrProtected(path.to_path_buf()));
        }
        let declared_size = file.size();
        if declared_size > limits.max_entry_bytes {
            return Err(ConvertError::EntryTooLarge {
                name,
                limit: limits.max_entry_bytes / MIB,
            });
        }
        if unpacked_bytes.saturating_add(declared_size) > limits.max_unpacked_bytes {
            return Err(ConvertError::UnpackedTooLarge {
                limit: limits.max_unpacked_bytes / MIB,
            });
        }
        unpacked_bytes += declared_size;
        if name.starts_with("BinData/") {
            image_bytes = image_bytes.saturating_add(declared_size);
        }
    }
    Ok(estimated_memory_bytes(unpacked_bytes, image_bytes))
}

pub fn estimated_memory_bytes(unpacked_bytes: u64, image_bytes: u64) -> u64 {
    unpacked_bytes
        .saturating_mul(3)
        .saturating_add(image_bytes.saturating_mul(4) / 3)
        .saturating_add(16 * MIB)
}

impl Package {
    pub fn open(path: impl AsRef<Path>, limits: &Limits) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(ConvertError::MissingInput(path));
        }
        let metadata = std::fs::metadata(&path)?;
        if metadata.len() > limits.max_input_bytes {
            return Err(ConvertError::InputTooLarge {
                limit: limits.max_input_bytes / MIB,
            });
        }
        let input = std::fs::read(&path)?;
        if input.len() < 4 || input[..4] != [0x50, 0x4b, 0x03, 0x04] {
            return Err(ConvertError::UnsupportedFormat(path));
        }
        let input_sha256 = sha256_hex(&input);
        let cursor = std::io::Cursor::new(input);
        let mut archive = ZipArchive::new(cursor)?;
        if archive.len() > limits.max_entries {
            return Err(ConvertError::TooManyEntries {
                limit: limits.max_entries,
            });
        }

        let mut entries = BTreeMap::new();
        let mut normalized_names = BTreeSet::new();
        let mut unpacked_bytes = 0u64;
        for index in 0..archive.len() {
            let mut file = archive.by_index(index)?;
            let raw_name = file.name().to_owned();
            let name = normalize_entry_name(&raw_name)?;
            if !normalized_names.insert(name.clone()) {
                return Err(ConvertError::DuplicateZipPath(name));
            }
            if file.encrypted() {
                return Err(ConvertError::EncryptedOrProtected(path));
            }
            let declared_size = file.size();
            if declared_size > limits.max_entry_bytes {
                return Err(ConvertError::EntryTooLarge {
                    name,
                    limit: limits.max_entry_bytes / MIB,
                });
            }
            if unpacked_bytes.saturating_add(declared_size) > limits.max_unpacked_bytes {
                return Err(ConvertError::UnpackedTooLarge {
                    limit: limits.max_unpacked_bytes / MIB,
                });
            }
            let mut data = Vec::with_capacity(declared_size.min(8 * MIB) as usize);
            file.read_to_end(&mut data)?;
            if data.len() as u64 != declared_size {
                return Err(ConvertError::InvalidValue {
                    path: raw_name,
                    message: "ZIP entry size changed while reading".to_owned(),
                });
            }
            unpacked_bytes += declared_size;
            entries.insert(name, data);
        }

        Ok(Self {
            path,
            input_sha256,
            entries,
            unpacked_bytes,
        })
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }

    pub fn required(&self, name: &str) -> Result<&[u8]> {
        self.get(name)
            .ok_or_else(|| ConvertError::MissingEntry(name.to_owned()))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn from_entries(entries: &[(&str, &[u8])]) -> Self {
        Self {
            path: PathBuf::new(),
            input_sha256: String::new(),
            entries: entries
                .iter()
                .map(|(name, data)| ((*name).to_owned(), data.to_vec()))
                .collect(),
            unpacked_bytes: 0,
        }
    }
}

pub fn normalize_entry_name(raw: &str) -> Result<String> {
    if raw.contains('\0') || raw.contains('\\') || raw.starts_with('/') {
        return Err(ConvertError::UnsafeZipPath(raw.to_owned()));
    }
    let mut result = Vec::new();
    for segment in raw.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || (result.is_empty() && segment.ends_with(':')) {
            return Err(ConvertError::UnsafeZipPath(raw.to_owned()));
        }
        result.push(segment);
    }
    if result.is_empty() {
        return Err(ConvertError::UnsafeZipPath(raw.to_owned()));
    }
    Ok(result.join("/"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
