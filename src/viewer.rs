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
    let sites_dir = workspace.join("sites");
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
            let sites_dir = sites_dir.clone();
            tokio::spawn(async move {
                let _ = handle(sock, store, page_path, sites_dir).await;
            });
        }
    }
}

/// Serve a static file from a per-project site directory, or None when the
/// request isn't for a subdomain site. A wildcard DNS record points every
/// `<name>.<apex>` at this server, and a site exists the moment an agent
/// writes `workspace/sites/<name>/index.html` — no founder step per project.
async fn try_site(
    sock: &mut TcpStream,
    sites_dir: &std::path::Path,
    host: &str,
    path: &str,
    hsts: &str,
) -> Option<std::io::Result<()>> {
    // Only hosts with a subdomain label beyond the apex (name.domain.tld),
    // and never www — the apex and www stay the company's own page.
    let host = host.split(':').next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 3 || labels[0] == "www" {
        return None;
    }
    let name = labels[0];
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let dir = sites_dir.join(name);
    if !dir.is_dir() {
        return None;
    }
    // Path sanitation: strip query, reject traversal, default to index.html.
    let path = path.split('?').next().unwrap_or("/");
    if path.contains("..") {
        return None;
    }
    let rel = path.trim_start_matches('/');
    let mut file = dir.join(if rel.is_empty() { "index.html" } else { rel });
    if file.is_dir() {
        file = file.join("index.html");
    }
    let body = match std::fs::read(&file) {
        Ok(b) => b,
        Err(_) => {
            return Some(
                sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await,
            )
        }
    };
    let mime = match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" | "md" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{hsts}Connection: close\r\n\r\n",
        body.len()
    );
    Some(async {
        sock.write_all(head.as_bytes()).await?;
        sock.write_all(&body).await
    }
    .await)
}

async fn handle(
    mut sock: TcpStream,
    store: Arc<Store>,
    page_path: PathBuf,
    sites_dir: PathBuf,
) -> std::io::Result<()> {
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

    // HSTS, but only on requests that actually arrived over TLS. Without it, typing
    // the bare domain always hits http:// first and leans on the redirect, so there
    // is an insecure hop every time. The proto check matters: this server speaks
    // plain HTTP (the platform terminates TLS and sets X-Forwarded-Proto), and
    // sending HSTS from a local `khan run` on http://localhost would be a nasty
    // surprise to pin into a developer's browser.
    let secure = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("x-forwarded-proto:") && l.contains("https")
    });
    // One year, no includeSubDomains: apex and www each get their own header when
    // visited, and nothing here should speak for subdomains that may not have TLS.
    let hsts = if secure { "Strict-Transport-Security: max-age=31536000\r\n" } else { "" };

    // Subdomain project sites first: <name>.<apex> serves workspace/sites/<name>/.
    let host = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("host:").map(|h| h.trim().to_string()))
        .unwrap_or_default();
    if let Some(res) = try_site(&mut sock, &sites_dir, &host, path, hsts).await {
        return res;
    }

    if path.starts_with("/logs") {
        // Subscribe before replaying history so no event can fall in the gap;
        // the frontend dedupes by row id.
        let mut rx = store.subscribe_log();
        sock.write_all(
            format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n{hsts}Connection: close\r\n\r\n")
                .as_bytes(),
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
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n{hsts}Connection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await?;
        sock.write_all(body.as_bytes()).await
    } else {
        sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
    }
}
