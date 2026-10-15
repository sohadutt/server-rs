use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::config::{Config, valid_mime_type};

#[derive(Debug)]
pub struct StaticFileServer {
    root: PathBuf,
    index_file: PathBuf,
    not_found_page: PathBuf,
    max_size: u64,
    default_mime_type: String,
    mime_types: HashMap<String, String>,
}

#[derive(Debug)]
pub struct StaticFile {
    pub data: Vec<u8>,
    pub content_type: String,
    pub relative_path: PathBuf,
}

impl StaticFileServer {
    pub fn new(config: &Config) -> Result<Self, String> {
        if !(1..=1024 * 1024 * 1024_u64).contains(&config.max_static_file_size_bytes) {
            return Err("Static file size limit must be between 1 byte and 1 GiB".into());
        }
        if !valid_mime_type(&config.default_mime_type) {
            return Err("Invalid default MIME type".into());
        }
        validate_relative(&config.index_file)?;
        validate_relative(&config.not_found_page)?;
        let root = fs::canonicalize(&config.document_root).map_err(|error| {
            format!(
                "Invalid document root '{}': {error}",
                config.document_root.display()
            )
        })?;
        if !root.is_dir() {
            return Err(format!(
                "Document root '{}' is not a directory",
                root.display()
            ));
        }
        let mime_types = load_mime_types(&config.mime_types_file)?;
        if mime_types.is_empty() || mime_types.len() > 128 {
            return Err("MIME types file must contain between 1 and 128 entries".into());
        }
        let server = Self {
            root,
            index_file: config.index_file.clone(),
            not_found_page: config.not_found_page.clone(),
            max_size: config.max_static_file_size_bytes,
            default_mime_type: config.default_mime_type.clone(),
            mime_types,
        };
        server
            .load_target("/")
            .map_err(|error| format!("Configured index file cannot be loaded: {error}"))?;
        server
            .not_found()
            .map_err(|error| format!("Configured 404 file cannot be loaded: {error}"))?;
        Ok(server)
    }

    pub fn load_target(&self, target: &str) -> Result<StaticFile, String> {
        let decoded = decode_request_path(target)?;
        let relative = if decoded.as_os_str().is_empty() {
            self.index_file.clone()
        } else {
            decoded
        };
        let mut path = self.resolve(&relative)?;
        if path.is_dir() {
            path = self.resolve(&relative.join(&self.index_file))?;
        }
        self.read_file(path)
    }

    pub fn not_found(&self) -> Result<StaticFile, String> {
        self.read_file(self.resolve(&self.not_found_page)?)
    }

    fn resolve(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("unsafe static path".into());
        }
        let resolved =
            fs::canonicalize(self.root.join(relative)).map_err(|error| error.to_string())?;
        if !resolved.starts_with(&self.root) {
            return Err("requested path escapes document root".into());
        }
        Ok(resolved)
    }

    fn read_file(&self, path: PathBuf) -> Result<StaticFile, String> {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("requested path is not a regular file".into());
        }
        if metadata.len() > self.max_size {
            return Err("file exceeds configured size limit".into());
        }
        let data = fs::read(&path).map_err(|error| error.to_string())?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let content_type = self
            .mime_types
            .get(&extension)
            .cloned()
            .unwrap_or_else(|| self.default_mime_type.clone());
        let relative_path = path.strip_prefix(&self.root).unwrap_or(&path).to_owned();
        log::debug!("Serving '{}' as {}", relative_path.display(), content_type);
        Ok(StaticFile {
            data,
            content_type,
            relative_path,
        })
    }
}

fn load_mime_types(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Cannot read MIME table '{}': {error}", path.display()))?;
    let mut result = HashMap::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line = raw
            .split_once('#')
            .map_or(raw, |(content, _)| content)
            .trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let extension = fields.next().unwrap_or_default();
        let media_type = fields.next().unwrap_or_default();
        if fields.next().is_some()
            || extension.is_empty()
            || extension.len() >= 32
            || !extension
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            || media_type.len() >= 128
            || !valid_mime_type(media_type)
        {
            return Err(format!(
                "{}:{}: expected '<extension> <media/type>'",
                path.display(),
                line_index + 1
            ));
        }
        result.insert(extension.to_ascii_lowercase(), media_type.to_owned());
        if result.len() > 128 {
            return Err("MIME table exceeds 128 entries".into());
        }
    }
    Ok(result)
}

fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "Invalid document-root relative path '{}':",
            path.display()
        ));
    }
    Ok(())
}

fn decode_request_path(target: &str) -> Result<PathBuf, String> {
    if !target.starts_with('/') {
        return Err("request target must start with '/'".into());
    }
    let encoded = target
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .as_bytes();
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut index = 1;
    while index < encoded.len() {
        let byte = if encoded[index] == b'%' {
            if index + 2 >= encoded.len() {
                return Err("malformed percent escape".into());
            }
            let high = hex(encoded[index + 1]).ok_or("malformed percent escape")?;
            let low = hex(encoded[index + 2]).ok_or("malformed percent escape")?;
            index += 2;
            high * 16 + low
        } else {
            encoded[index]
        };
        if byte == 0 || byte == b'\\' || byte < 0x20 || byte == 0x7f {
            return Err("invalid path character".into());
        }
        decoded.push(byte);
        index += 1;
    }
    let decoded = String::from_utf8(decoded).map_err(|_| "request path is not UTF-8")?;
    if decoded
        .split('/')
        .any(|component| component == "." || component == "..")
    {
        return Err("path traversal is not allowed".into());
    }
    Ok(PathBuf::from(decoded.trim_matches('/')))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
