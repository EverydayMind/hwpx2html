use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ConvertError>;

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("input file does not exist: {0}")]
    MissingInput(PathBuf),
    #[error("input is not a ZIP-HWPX file (expected PK signature): {0}")]
    UnsupportedFormat(PathBuf),
    #[error("encrypted or protected document: {0}")]
    EncryptedOrProtected(PathBuf),
    #[error("ZIP entry path is unsafe: {0}")]
    UnsafeZipPath(String),
    #[error("duplicate normalized ZIP entry path: {0}")]
    DuplicateZipPath(String),
    #[error("ZIP entry count exceeds the configured limit ({limit})")]
    TooManyEntries { limit: usize },
    #[error("ZIP entry exceeds the configured limit ({limit} MiB): {name}")]
    EntryTooLarge { name: String, limit: u64 },
    #[error("unpacked ZIP content exceeds the configured limit ({limit} MiB)")]
    UnpackedTooLarge { limit: u64 },
    #[error("input file exceeds the configured limit ({limit} MiB)")]
    InputTooLarge { limit: u64 },
    #[error("memory budget exceeded: estimated {estimated} MiB, budget {budget} MiB")]
    MemoryBudgetExceeded { estimated: u64, budget: u64 },
    #[error("required HWPX entry is missing: {0}")]
    MissingEntry(String),
    #[error("unsupported HWPX schema: {0}")]
    UnsupportedSchema(String),
    #[error("XML parse error in {path}: {message}")]
    Xml { path: String, message: String },
    #[error("invalid HWPX value in {path}: {message}")]
    InvalidValue { path: String, message: String },
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("strict mode rejected unsupported objects")]
    StrictUnsupported,
    #[error("the document has content the writer cannot write yet: {0}")]
    UnwritableContent(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
