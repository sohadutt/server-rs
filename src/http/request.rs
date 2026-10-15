use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::{Duration, timeout},
};

pub const MAX_REQUEST_BYTES: usize = 4095;

#[derive(Debug)]
pub enum RequestError {
    Io(std::io::Error),
    TooLarge,
    Malformed,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::TooLarge => formatter.write_str("request exceeds 4095 bytes"),
            Self::Malformed => formatter.write_str("malformed HTTP request"),
        }
    }
}

impl std::error::Error for RequestError {}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub target: String,
    pub version: String,
    pub raw: Vec<u8>,
}

pub async fn read_request(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<Option<HttpRequest>, RequestError> {
    let mut raw = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .map_err(|_| {
                RequestError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "request read timed out",
                ))
            })?
            .map_err(RequestError::Io)?;
        if read == 0 {
            return if raw.is_empty() {
                Ok(None)
            } else {
                Err(RequestError::Malformed)
            };
        }
        if raw.len() + read > MAX_REQUEST_BYTES {
            return Err(RequestError::TooLarge);
        }
        raw.extend_from_slice(&chunk[..read]);
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = end + 4;
            break;
        }
        if raw.len() > MAX_REQUEST_BYTES - 4 {
            return Err(RequestError::TooLarge);
        }
    }
    let headers = std::str::from_utf8(&raw[..header_end]).map_err(|_| RequestError::Malformed)?;
    let content_length = headers
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let total_length = header_end
        .checked_add(content_length)
        .ok_or(RequestError::TooLarge)?;
    if total_length > MAX_REQUEST_BYTES {
        return Err(RequestError::TooLarge);
    }
    while raw.len() < total_length {
        let take = (total_length - raw.len()).min(chunk.len());
        let read = timeout(Duration::from_secs(5), stream.read(&mut chunk[..take]))
            .await
            .map_err(|_| {
                RequestError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "request body timed out",
                ))
            })?
            .map_err(RequestError::Io)?;
        if read == 0 {
            return Err(RequestError::Malformed);
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    raw.truncate(total_length);
    parse_request(raw).map(Some)
}

fn parse_request(raw: Vec<u8>) -> Result<HttpRequest, RequestError> {
    let line_end = raw
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .unwrap_or(raw.len());
    let first_line = &raw[..line_end];
    let line = std::str::from_utf8(first_line)
        .map_err(|_| RequestError::Malformed)?
        .trim_end_matches('\r');
    let mut parts = line.split_ascii_whitespace();
    let method = parts.next().ok_or(RequestError::Malformed)?;
    let target = parts.next().ok_or(RequestError::Malformed)?;
    let version = parts.next().ok_or(RequestError::Malformed)?;
    if parts.next().is_some()
        || method.is_empty()
        || method.len() >= 16
        || target.len() >= 2048
        || !target.starts_with('/')
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
    {
        return Err(RequestError::Malformed);
    }
    Ok(HttpRequest {
        method: method.to_owned(),
        target: target.to_owned(),
        version: version.to_owned(),
        raw,
    })
}
