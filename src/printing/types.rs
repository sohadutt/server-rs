//! Public printer, print-job, and document types.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrinterState {
    Idle,
    Processing,
    Stopped,
    #[default]
    Unknown,
}

impl PrinterState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Processing => "processing",
            Self::Stopped => "stopped",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Printer {
    pub id: usize,
    pub name: String,
    pub instance: Option<String>,
    pub uri: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub make_and_model: Option<String>,
    pub state: PrinterState,
    pub state_message: Option<String>,
    pub is_default: bool,
    pub is_accepting_jobs: bool,
    pub is_shared: bool,
    pub is_busy: bool,
    pub queued_jobs: usize,
    pub supports_color: bool,
    pub supports_duplex: bool,
    pub is_discovered: bool,
    pub supported_media: Vec<String>,
    pub resolution_x: i32,
    pub resolution_y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrintJobState {
    #[default]
    Unknown,
    Pending,
    Held,
    Processing,
    Completed,
    Canceled,
    Aborted,
}

#[derive(Clone, Debug, Default)]
pub struct PrintJob {
    pub id: i32,
    pub printer_name: Option<String>,
    pub title: Option<String>,
    pub owner: Option<String>,
    pub file_path: Option<PathBuf>,
    pub state: PrintJobState,
    pub size_bytes: u64,
    pub impressions_completed: i32,
}

#[derive(Clone, Debug)]
pub struct PrintDocument {
    pub path: PathBuf,
    pub mime_type: String,
    pub size_bytes: u64,
}

impl PrintDocument {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = fs::canonicalize(path.as_ref())
            .map_err(|error| format!("Cannot open print document: {error}"))?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("Print document is not a regular file".into());
        }
        if metadata.len() == 0 {
            return Err("Print document is empty".into());
        }
        let mime_type = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        Ok(Self {
            path,
            mime_type,
            size_bytes: metadata.len(),
        })
    }

    pub fn validate(&self, max_size_bytes: u64) -> Result<(), String> {
        let metadata = fs::metadata(&self.path)
            .map_err(|error| format!("Print document is not available: {error}"))?;
        if !metadata.is_file() {
            return Err("Print document is not a regular file".into());
        }
        if metadata.len() == 0 {
            return Err("Print document is empty".into());
        }
        if metadata.len() > max_size_bytes {
            return Err(format!("Print document exceeds {max_size_bytes} bytes"));
        }
        Ok(())
    }
}
