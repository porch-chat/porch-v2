//! Explicit, opt-in link metadata. Never return remote HTML or image URLs.
use crate::{
    identity::{ApiError, IdentityState, authenticate},
    messaging::context,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::post,
};
use futures_util::StreamExt;
use porch_protocol::{EmbedRequest, ErrorCode, LinkEmbed, permissions::READ_HISTORY};
use reqwest::Url;
use scraper::{Html, Selector};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
type Result<T> = std::result::Result<T, ApiError>;
fn invalid() -> ApiError {
    ApiError(ErrorCode::InvalidRequest)
}
fn limited() -> ApiError {
    ApiError(ErrorCode::RateLimited)
}

#[derive(Clone)]
pub struct EmbedService {
    pool: PgPool,
    identity: IdentityState,
    work: Arc<Semaphore>,
    rates: Arc<Mutex<HashMap<i64, (Instant, u32)>>>,
}
impl EmbedService {
    pub fn new(pool: PgPool, identity: IdentityState) -> Self {
        Self {
            pool,
            identity,
            work: Arc::new(Semaphore::new(4)),
            rates: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn routes(self) -> Router {
        Router::new()
            .route(
                "/conversations/{conversation}/messages/{message}/embed",
                post(preview),
            )
            .layer(axum::extract::DefaultBodyLimit::max(4096))
            .layer(axum::middleware::map_response(
                async |mut response: axum::response::Response| {
                    response
                        .headers_mut()
                        .insert("cache-control", "no-store".parse().unwrap());
                    response
                        .headers_mut()
                        .insert("x-content-type-options", "nosniff".parse().unwrap());
                    response
                },
            ))
            .with_state(self)
    }
    async fn rate(&self, user: i64) -> Result<()> {
        let mut rates = self.rates.lock().await;
        rates.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(60));
        if rates.len() >= 1024 && !rates.contains_key(&user) {
            return Err(limited());
        }
        // Key zero is a global 60/minute budget, positive IDs are six/minute.
        for (key, max) in [(0, 60), (user, 6)] {
            let entry = rates.entry(key).or_insert((Instant::now(), 0));
            if entry.1 >= max {
                return Err(limited());
            }
            entry.1 += 1;
        }
        Ok(())
    }
    async fn readable(&self, user: i64, conversation: i64, message: i64, url: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        context(&mut tx, conversation, user, true)
            .await?
            .require(READ_HISTORY)?;
        let source: Option<String> = sqlx::query_scalar(
            "SELECT source FROM messages WHERE conversation_id=$1 AND id=$2 AND NOT deleted",
        )
        .bind(conversation)
        .bind(message)
        .fetch_optional(&mut *tx)
        .await?;
        let source = source.ok_or(ApiError(ErrorCode::NotFound))?;
        if !contains_link(&source, url) {
            return Err(invalid());
        }
        tx.commit().await?;
        Ok(())
    }
}
fn contains_link(source: &str, url: &str) -> bool {
    pulldown_cmark::Parser::new(source).any(|e| matches!(e,
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {dest_url,..}) if dest_url.as_ref()==url))
}
fn checked_url(raw: &str) -> Result<Url> {
    if raw.len() > 2048 || raw.chars().any(char::is_control) {
        return Err(invalid());
    }
    let u = Url::parse(raw).map_err(|_| invalid())?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || u.port_or_known_default() != Some(443)
        || u.host_str().is_none()
    {
        return Err(invalid());
    }
    Ok(u)
}
/// Conservative IPv4-only policy. All IPv6 including mapped/transition forms is
/// rejected. IANA special-purpose blocks are denied even with public exceptions.
fn public(ip: IpAddr) -> bool {
    let IpAddr::V4(v) = ip else {
        return false;
    };
    let [a, b, c, _] = v.octets();
    !(matches!(a, 0 | 10 | 127)
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && ((b == 0 && (c == 0 || c == 2)) || (b == 168) || (b == 88 && c == 99)))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}
async fn fetch(url: Url) -> Result<Vec<u8>> {
    let host = url.host_str().ok_or_else(invalid)?;
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| invalid())?
        .collect();
    checked_addresses(&addrs)?;
    // Pin ALL vetted answers. No second unchecked DNS resolution or redirects.
    let client = reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .resolve_to_addrs(host, &addrs)
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|_| invalid())?;
    let response = client
        .get(url)
        .header("accept", "text/html")
        .header("accept-encoding", "identity")
        .send()
        .await
        .map_err(|_| invalid())?;
    read_body(response).await
}
fn checked_addresses(addrs: &[SocketAddr]) -> Result<()> {
    if addrs.is_empty()
        || addrs.len() > 32
        || addrs.iter().any(|a| !public(a.ip()) || a.port() != 443)
    {
        return Err(invalid());
    }
    Ok(())
}
async fn read_body(response: reqwest::Response) -> Result<Vec<u8>> {
    let h = response.headers();
    if !response.status().is_success()
        || response.content_length().is_some_and(|n| n > 1024 * 1024)
        || h.get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(';').next())
            .is_none_or(|s| !s.trim().eq_ignore_ascii_case("text/html"))
        || h.get("content-encoding").is_some_and(|v| v != "identity")
    {
        return Err(invalid());
    }
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| invalid())?;
        if data.len() + chunk.len() > 1024 * 1024 {
            return Err(invalid());
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
fn clean(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}
fn metadata(url: String, data: Vec<u8>) -> LinkEmbed {
    let html = Html::parse_document(&String::from_utf8_lossy(&data));
    let title = Selector::parse("title").unwrap();
    let meta = Selector::parse("meta").unwrap();
    let mut out = LinkEmbed {
        url,
        title: String::new(),
        description: String::new(),
    };
    for e in html.select(&meta) {
        let key = e
            .value()
            .attr("property")
            .or_else(|| e.value().attr("name"))
            .unwrap_or("");
        let val = e.value().attr("content").unwrap_or("");
        if key.eq_ignore_ascii_case("og:title") && out.title.is_empty() {
            out.title = clean(val, 200);
        }
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "og:description" | "description"
        ) && out.description.is_empty()
        {
            out.description = clean(val, 500);
        }
    }
    if out.title.is_empty() {
        out.title = clean(
            &html
                .select(&title)
                .next()
                .map(|e| e.text().collect::<String>())
                .unwrap_or_default(),
            200,
        );
    }
    out
}
async fn preview(
    State(s): State<EmbedService>,
    h: HeaderMap,
    Path((conversation, message)): Path<(i64, i64)>,
    Json(request): Json<EmbedRequest>,
) -> Result<([(&'static str, &'static str); 2], Json<LinkEmbed>)> {
    let (user, _) = authenticate(&s.identity, &h).await?;
    let url = checked_url(&request.url)?;
    s.readable(user.id, conversation, message, &request.url)
        .await?;
    s.rate(user.id).await?;
    let permit = s.work.clone().try_acquire_owned().map_err(|_| limited())?;
    let data = tokio::time::timeout(Duration::from_secs(5), fetch(url))
        .await
        .map_err(|_| invalid())??;
    let raw = request.url.clone();
    let embed = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        metadata(raw, data)
    })
    .await
    .map_err(|_| invalid())?;
    authenticate(&s.identity, &h).await?;
    s.readable(user.id, conversation, message, &request.url)
        .await?;
    Ok((
        [
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        Json(embed),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deny_special_and_transitional_addresses() {
        for ip in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.9",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::",
            "2001:4860:4860::8888",
        ] {
            assert!(!public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "100.128.0.1", "172.32.0.1"] {
            assert!(public(ip.parse().unwrap()));
        }
    }
    #[test]
    fn mixed_dns_answers_are_never_partially_trusted() {
        let good = "8.8.8.8:443".parse().unwrap();
        let bad = "127.0.0.1:443".parse().unwrap();
        assert!(checked_addresses(&[good]).is_ok());
        for addrs in [
            vec![],
            vec![good, bad],
            vec![bad, good],
            vec![good; 33],
            vec!["8.8.8.8:80".parse().unwrap()],
        ] {
            assert!(checked_addresses(&addrs).is_err());
        }
    }
    #[tokio::test]
    async fn response_status_mime_encoding_and_stream_bounds() {
        use axum::http::Response;
        let make = |status, mime, encoding, data: Vec<u8>| {
            reqwest::Response::from(
                Response::builder()
                    .status(status)
                    .header("content-type", mime)
                    .header("content-encoding", encoding)
                    .body(data)
                    .unwrap(),
            )
        };
        assert!(
            read_body(make(
                200,
                "text/html; charset=utf-8",
                "identity",
                b"<title>ok</title>".to_vec()
            ))
            .await
            .is_ok()
        );
        for (status, mime, encoding) in [
            (302, "text/html", "identity"),
            (404, "text/html", "identity"),
            (200, "image/svg+xml", "identity"),
            (200, "application/json", "identity"),
            (200, "text/html", "gzip"),
        ] {
            assert!(
                read_body(make(status, mime, encoding, vec![]))
                    .await
                    .is_err()
            );
        }
        assert!(
            read_body(make(200, "text/html", "identity", vec![0; 1024 * 1024 + 1]))
                .await
                .is_err()
        );
        let streamed = reqwest::Body::wrap_stream(futures_util::stream::iter([
            Ok::<_, std::io::Error>(vec![b'a'; 600_000]),
            Ok(vec![b'b'; 600_000]),
        ]));
        let response = reqwest::Response::from(
            Response::builder()
                .header("content-type", "text/html")
                .body(streamed)
                .unwrap(),
        );
        assert!(read_body(response).await.is_err());
    }
    #[test]
    fn only_explicit_safe_urls_and_plain_metadata() {
        for url in [
            "http://example.com",
            "https://u:p@example.com",
            "https://example.com:444",
            "https://example.com/#frag",
            "file:///etc/passwd",
        ] {
            assert!(checked_url(url).is_err());
        }
        assert!(checked_url("https://example.com/foo?q=1").is_ok());
        assert!(contains_link(
            "[test](https://example.com)",
            "https://example.com"
        ));
        assert!(!contains_link(
            "https://example.com.evil",
            "https://example.com"
        ));
        let m=metadata("https://example.com".into(),b"<title> Safe &amp; plain </title><meta name='description' content='hello'><script>alert(1)</script><meta property='og:image' content='http://127.0.0.1'>".to_vec());
        assert_eq!(m.title, "Safe & plain");
        assert_eq!(m.description, "hello");
    }
}
