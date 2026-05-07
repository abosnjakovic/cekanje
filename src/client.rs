use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn http_get(port: u16, path: &str) -> Result<String> {
    let mut stream = connect(port).await?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await?;
    Ok(split_body(&raw).to_string())
}

pub async fn http_post_json(port: u16, path: &str, body: &str) -> Result<String> {
    let mut stream = connect(port).await?;
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {len}\r\n\r\n{body}",
        len = body.len(),
    );
    stream.write_all(req.as_bytes()).await?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await?;
    Ok(split_body(&raw).to_string())
}

async fn connect(port: u16) -> Result<TcpStream> {
    TcpStream::connect(("127.0.0.1", port))
        .await
        .with_context(|| format!("connect to cekanje daemon on 127.0.0.1:{port}"))
}

fn split_body(raw: &str) -> &str {
    raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")
}
