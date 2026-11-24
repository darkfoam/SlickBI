#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The `.scz` workbook container.
//!
//! A `.scz` is a zip archive that holds one [`Workbook`] document plus the data it needs to be
//! self-contained. The layout is:
//!
//! ```text
//! workbook.json        the document, in the versioned JSON format
//! data/<table>.parquet optional extracts, one per table, named after the table
//! thumbs/<id>.png      optional previews (written from 0.0.2)
//! ```
//!
//! The document is the single source of truth for everything the application shows; the extracts
//! are read-only snapshots of source data, so a workbook can reopen without the original files.
//! Nothing else may live in the archive, and opening refuses a document written by a newer
//! format, so a `.scz` is never partially loaded.
//!
//! The document is serialised by [`slickbi_spec`], which owns the format, the version and the
//! invariants. This crate only packages and unpacks it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use slickbi_spec::{FormatError, Workbook};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// The name of the entry that holds the workbook document.
pub const WORKBOOK_ENTRY: &str = "workbook.json";

/// A loaded or about-to-be-saved `.scz` workbook.
#[derive(Debug, Clone, PartialEq)]
pub struct Scz {
    workbook: Workbook,
    extracts: BTreeMap<String, Vec<u8>>,
}

impl Scz {
    /// Wraps a workbook with no embedded data.
    pub fn new(workbook: Workbook) -> Self {
        Scz {
            workbook,
            extracts: BTreeMap::new(),
        }
    }

    /// Adds a Parquet extract for the named table.
    ///
    /// The name is the table's SQL name; the bytes are stored at `data/<name>.parquet`.
    pub fn with_extract(mut self, table: impl Into<String>, parquet: Vec<u8>) -> Self {
        self.extracts.insert(table.into(), parquet);
        self
    }

    /// The workbook document.
    pub fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    /// The document, consuming the container.
    pub fn into_workbook(self) -> Workbook {
        self.workbook
    }

    /// The Parquet bytes for a table, if the workbook embeds them.
    pub fn extract(&self, table: &str) -> Option<&[u8]> {
        self.extracts.get(table).map(Vec::as_slice)
    }

    /// Every embedded extract, as table name and bytes.
    pub fn extracts(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.extracts
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
    }

    /// Serialises the workbook to an in-memory `.scz`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbookError`] if the document cannot be serialised.
    pub fn to_bytes(&self) -> Result<Vec<u8>, WorkbookError> {
        let cursor = Cursor::new(Vec::new());
        let cursor = self.write_into(cursor)?;
        Ok(cursor.into_inner())
    }

    /// Writes the workbook to a `.scz` file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbookError`] if the file cannot be created or written.
    pub fn save(&self, path: &Path) -> Result<(), WorkbookError> {
        let file = File::create(path)?;
        self.write_into(file)?;
        Ok(())
    }

    fn write_into<W: Write + Seek>(&self, writer: W) -> Result<W, WorkbookError> {
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default();

        let json = self.workbook.to_json_string()?;
        zip.start_file(WORKBOOK_ENTRY, options)?;
        zip.write_all(json.as_bytes())?;

        for (table, bytes) in &self.extracts {
            zip.start_file(format!("data/{table}.parquet"), options)?;
            zip.write_all(bytes)?;
        }

        Ok(zip.finish()?)
    }

    /// Parses a `.scz` from bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbookError`] if the archive is not a zip, lacks `workbook.json`, holds a
    /// document this build cannot read, or has an extract entry outside `data/`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Scz, WorkbookError> {
        let archive = ZipArchive::new(Cursor::new(bytes))?;
        read(archive)
    }

    /// Opens a `.scz` file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbookError`] for the same reasons as [`Scz::from_bytes`], plus filesystem
    /// errors.
    pub fn open(path: &Path) -> Result<Scz, WorkbookError> {
        let file = File::open(path)?;
        let archive = ZipArchive::new(file)?;
        read(archive)
    }
}

fn read<R: Read + Seek>(mut archive: ZipArchive<R>) -> Result<Scz, WorkbookError> {
    let mut workbook_json = None;
    let mut extracts = BTreeMap::new();

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.into_owned();

        if name == WORKBOOK_ENTRY {
            let mut json = String::new();
            entry.read_to_string(&mut json)?;
            workbook_json = Some(json);
        } else if let Some(table) = name.strip_prefix("data/") {
            let Some(table) = table.strip_suffix(".parquet") else {
                continue;
            };
            if table.is_empty() || table.contains('/') || table.contains("..") {
                return Err(WorkbookError::InvalidEntry(name));
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            extracts.insert(table.to_owned(), bytes);
        }
        // Any other entry (thumbs/, future additions) is ignored.
    }

    let json = workbook_json.ok_or(WorkbookError::MissingWorkbook)?;
    let workbook = Workbook::load_str(&json)?;

    Ok(Scz { workbook, extracts })
}

/// Anything that can go wrong while reading or writing a `.scz`.
#[derive(Debug, Error)]
pub enum WorkbookError {
    /// A filesystem operation failed.
    #[error("cannot read or write the workbook: {0}")]
    Io(#[from] std::io::Error),
    /// The archive is not a valid zip.
    #[error("the .scz archive is malformed: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// The document is not a valid SlickBI workbook.
    #[error("the workbook document is invalid: {0}")]
    Format(#[from] FormatError),
    /// The archive has no `workbook.json`.
    #[error("the .scz archive has no {WORKBOOK_ENTRY} entry")]
    MissingWorkbook,
    /// An extract entry is outside `data/` or malformed.
    #[error("the .scz archive has an invalid entry: '{0}'")]
    InvalidEntry(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use slickbi_spec::{WorkbookId, WorkbookMeta};

    fn workbook() -> Workbook {
        Workbook::new(WorkbookId::new("w1"), WorkbookMeta::new("Test"))
    }

    #[test]
    fn a_workbook_round_trips_through_bytes() {
        let source = workbook();
        let bytes = Scz::new(source.clone()).to_bytes().expect("serialises");
        let opened = Scz::from_bytes(&bytes).expect("parses");
        assert_eq!(opened.workbook(), &source);
    }

    #[test]
    fn an_extract_round_trips_with_its_table_name() {
        let scz = Scz::new(workbook()).with_extract("sales", vec![1, 2, 3, 4]);
        let bytes = scz.to_bytes().expect("serialises");
        let opened = Scz::from_bytes(&bytes).expect("parses");
        assert_eq!(opened.extract("sales"), Some(&[1, 2, 3, 4][..]));
        assert_eq!(opened.extract("other"), None);
    }

    #[test]
    fn a_file_round_trips_to_disk() {
        let path = std::env::temp_dir().join(format!("slickbi-{}.scz", std::process::id()));
        let source = workbook();
        Scz::new(source.clone()).save(&path).expect("saves");
        let opened = Scz::open(&path).expect("opens");
        assert_eq!(opened.workbook(), &source);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_archive_without_a_workbook_is_rejected() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("notes.txt", SimpleFileOptions::default())
            .expect("starts");
        zip.write_all(b"hello").expect("writes");
        let bytes = zip.finish().expect("finishes").into_inner();

        let error = Scz::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(error, WorkbookError::MissingWorkbook));
    }

    #[test]
    fn a_future_format_version_is_refused_by_the_spec() {
        // The container unpacks fine; the spec refuses the document version.
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(WORKBOOK_ENTRY, SimpleFileOptions::default())
            .expect("starts");
        zip.write_all(br#"{"format_version": 999, "id": "w1", "meta": {"name": "x"}}"#)
            .expect("writes");
        let bytes = zip.finish().expect("finishes").into_inner();

        let error = Scz::from_bytes(&bytes).expect_err("must fail");
        match error {
            WorkbookError::Format(FormatError::UnsupportedVersion { found, .. }) => {
                assert_eq!(found, 999);
            }
            other => panic!("expected a version error, got {other}"),
        }
    }

    #[test]
    fn a_malformed_extract_entry_is_rejected() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(WORKBOOK_ENTRY, SimpleFileOptions::default())
            .expect("starts");
        zip.write_all(workbook().to_json_string().expect("json").as_bytes())
            .expect("writes");
        zip.start_file("data/../evil.parquet", SimpleFileOptions::default())
            .expect("starts");
        zip.write_all(b"bad").expect("writes");
        let bytes = zip.finish().expect("finishes").into_inner();

        let error = Scz::from_bytes(&bytes).expect_err("must fail");
        assert!(matches!(error, WorkbookError::InvalidEntry(_)));
    }
}
