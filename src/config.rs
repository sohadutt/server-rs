use std::{fs, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub worker_threads: usize,
    pub document_root: PathBuf,
    pub index_file: PathBuf,
    pub not_found_page: PathBuf,
    pub mime_types_file: PathBuf,
    pub default_mime_type: String,
    pub max_static_file_size_bytes: u64,
    pub log_level: LogLevel,
    pub log_console: bool,
    pub log_file: bool,
    pub log_directory: PathBuf,
    pub cups_server: Option<String>,
    pub cups_encryption_required: bool,
}

impl Config {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("Cannot read config '{}': {error}", path.display()))?;
        let mut config = Self::default();
        for (line_index, raw) in text.lines().enumerate() {
            let line_no = line_index + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!(
                    "{}:{line_no}: expected key = value",
                    path.display()
                ));
            };
            config.set(key.trim(), value.trim()).map_err(|_| {
                format!(
                    "{}:{line_no}: invalid or unknown setting '{}'",
                    path.display(),
                    key.trim()
                )
            })?;
        }
        Ok(config)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), ()> {
        match key {
            "host" if !value.is_empty() => self.host = value.to_owned(),
            "port" => self.port = parse_int(value, 1, 65535).ok_or(())? as u16,
            "worker_threads" => self.worker_threads = parse_int(value, 1, 1024).ok_or(())? as usize,
            "document_root" if !value.is_empty() => self.document_root = value.into(),
            "index_file" if safe_relative(value) => self.index_file = value.into(),
            "not_found_page" if safe_relative(value) => self.not_found_page = value.into(),
            "mime_types_file" if !value.is_empty() => self.mime_types_file = value.into(),
            "default_mime_type" if valid_mime_type(value) => self.default_mime_type = value.into(),
            "max_static_file_size_mb" => {
                self.max_static_file_size_bytes = parse_int(value, 1, 1024).ok_or(())? * 1024 * 1024
            }
            "cups_server" if !value.is_empty() => self.cups_server = Some(value.to_owned()),
            "cups_encryption_required" => {
                self.cups_encryption_required = parse_bool(value).ok_or(())?
            }
            "log_level" => {
                self.log_level = match value {
                    "debug" => LogLevel::Debug,
                    "info" => LogLevel::Info,
                    "warn" => LogLevel::Warn,
                    "error" => LogLevel::Error,
                    _ => return Err(()),
                }
            }
            "log_console" => self.log_console = parse_bool(value).ok_or(())?,
            "log_file" => self.log_file = parse_bool(value).ok_or(())?,
            "log_directory" if !value.is_empty() => self.log_directory = value.into(),
            _ => return Err(()),
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
            worker_threads: 4,
            document_root: "public".into(),
            index_file: "index.html".into(),
            not_found_page: "404.html".into(),
            mime_types_file: "local/mime.types".into(),
            default_mime_type: "application/octet-stream".into(),
            max_static_file_size_bytes: 64 * 1024 * 1024,
            log_level: LogLevel::Info,
            log_console: true,
            log_file: true,
            log_directory: "local/logs".into(),
            cups_server: None,
            cups_encryption_required: false,
        }
    }
}

fn parse_int(value: &str, min: u64, max: u64) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|number| (min..=max).contains(number))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

pub fn valid_mime_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&^_.+-/".contains(&byte))
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.bytes().any(|b| b < 0x20 || b == 0x7f)
        })
}
