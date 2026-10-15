use tokio::io::{AsyncWrite, AsyncWriteExt};

pub async fn write_response(
    stream: &mut (impl AsyncWrite + Unpin),
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<usize> {
    if reason.contains(['\r', '\n']) || content_type.contains(['\r', '\n']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid response header",
        ));
    }
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(headers.len() + body.len())
}
