use crate::state::Store;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PAGE: &str = include_str!("viewer.html");

/// Minimal HTTP server for the live log viewer: `/` serves the page,
/// `/logs` is an SSE stream (recent history replay, then live events).
///
/// The page is served from workspace/viewer.html (seeded from the built-in
/// page on first boot) so agents can redesign the frontend freely with their
/// normal file tools — changes go live on the next page load. The server has
/// NO write endpoints: viewers can never send anything to the company.
pub async fn serve(store: Arc<Store>, port: u16, workspace: PathBuf) {
    let page_path = workspace.join("viewer.html");
    if !page_path.exists() {
        let _ = std::fs::write(&page_path, PAGE);
    }
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[viewer] cannot bind port {port}: {e} — log viewer disabled");
            return;
        }
    };
    println!("log viewer: http://localhost:{port}");
    loop {
        if let Ok((sock, _)) = listener.accept().await {
            let store = store.clone();
            let page_path = page_path.clone();
            tokio::spawn(async move {
                let _ = handle(sock, store, page_path).await;
            });
        }
    }
}

async fn handle(mut sock: TcpStream, store: Arc<Store>, page_path: PathBuf) -> std::io::Result<()> {
    let mut req = Vec::new();
    let mut buf = [0u8; 2048];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") && req.len() < 8192 {
        let n = sock.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        req.extend_from_slice(&buf[..n]);
    }
    let head = String::from_utf8_lossy(&req);
    let path = head.split_whitespace().nth(1).unwrap_or("/");

    if path.starts_with("/logs") {
        // Subscribe before replaying history so no event can fall in the gap;
        // the frontend dedupes by row id.
        let mut rx = store.subscribe_log();
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .await?;
        for row in store.log_tail(300) {
            sock.write_all(format!("data: {row}\n\n").as_bytes()).await?;
        }
        loop {
            match tokio::time::timeout(Duration::from_secs(20), rx.recv()).await {
                Ok(Ok(row)) => sock.write_all(format!("data: {row}\n\n").as_bytes()).await?,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(_)) => break,
                // Comment keepalive during quiet periods; a failed write means the
                // client is gone and the connection gets reaped.
                Err(_) => sock.write_all(b": keepalive\n\n").await?,
            }
        }
        Ok(())
    } else if path == "/" {
        // Read per-request so agent edits to the page show up on refresh.
        let body = std::fs::read_to_string(&page_path).unwrap_or_else(|_| PAGE.to_string());
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(head.as_bytes()).await?;
        sock.write_all(body.as_bytes()).await
    } else {
        sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
    }
}
