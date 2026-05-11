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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn split_body_returns_text_after_double_crlf() {
        assert_eq!(
            split_body("HTTP/1.1 200 OK\r\nHeader: v\r\n\r\nhello"),
            "hello"
        );
    }

    #[test]
    fn split_body_returns_empty_when_no_separator() {
        assert_eq!(split_body("HTTP/1.1 200 OK\r\nno body here"), "");
    }

    #[test]
    fn split_body_returns_empty_when_separator_at_end() {
        assert_eq!(split_body("HTTP/1.1 200 OK\r\n\r\n"), "");
    }

    /// Start a TCP server that returns `response` to one connection, optionally
    /// capturing the raw request bytes into the returned channel.
    async fn spawn_server(
        response: &'static str,
    ) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // Read the request bytes until the client signals end-of-write
            // (it shuts down after sending headers + body).
            let mut buf = Vec::new();
            // Read in a loop until we see header terminator; can't rely on
            // EOF because the client uses Connection: close on its end.
            let mut tmp = [0u8; 4096];
            loop {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    // For POSTs the body follows headers; give a brief read
                    // chance, then stop.
                    if let Some(idx) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = &buf[..idx];
                        let len = std::str::from_utf8(header)
                            .ok()
                            .and_then(|h| {
                                h.lines()
                                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                                    .and_then(|l| l.split(':').nth(1))
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        let body_so_far = buf.len() - (idx + 4);
                        if body_so_far >= len {
                            break;
                        }
                    }
                }
            }
            sock.write_all(response.as_bytes()).await.unwrap();
            sock.shutdown().await.ok();
            let _ = tx.send(buf);
        });
        (port, rx)
    }

    #[tokio::test]
    async fn http_get_returns_body() {
        let (port, _rx) = spawn_server("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").await;
        let body = http_get(port, "/status").await.unwrap();
        assert_eq!(body, "hello");
    }

    #[tokio::test]
    async fn http_get_sends_expected_request_line() {
        let (port, rx) = spawn_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        http_get(port, "/list").await.unwrap();
        let req = String::from_utf8(rx.await.unwrap()).unwrap();
        assert!(req.starts_with("GET /list HTTP/1.1\r\n"));
        assert!(req.contains("Host: 127.0.0.1\r\n"));
        assert!(req.contains("Connection: close\r\n"));
    }

    #[tokio::test]
    async fn http_post_json_writes_content_length_and_body() {
        let body = r#"{"pane":"%1"}"#;
        let (port, rx) = spawn_server("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
        let resp = http_post_json(port, "/visit", body).await.unwrap();
        assert_eq!(resp, "ok");
        let req = String::from_utf8(rx.await.unwrap()).unwrap();
        assert!(req.starts_with("POST /visit HTTP/1.1\r\n"));
        assert!(req.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert!(req.contains("Content-Type: application/json\r\n"));
        assert!(req.ends_with(body));
    }

    #[tokio::test]
    async fn http_get_returns_err_when_daemon_unreachable() {
        // Port 1 is reserved & not bound; connect should fail.
        let err = http_get(1, "/status").await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("connect to cekanje daemon"));
    }
}
