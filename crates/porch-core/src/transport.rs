use crate::{Error, Result};
use futures_util::StreamExt;
use porch_protocol::*;
use reqwest::{Client, Method, Url};
use serde::{Serialize, de::DeserializeOwned};
use std::time::Duration;

#[derive(Clone)]
pub struct Api {
    pub(crate) client: Client,
    origin: String,
    token: String,
}
impl Api {
    pub fn new(origin: &str, token: String) -> Result<Self> {
        let url = Url::parse(origin).map_err(|_| Error::InvalidData)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
        {
            return Err(Error::InvalidData);
        }
        if url.scheme() == "http"
            && !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        {
            return Err(Error::InvalidData);
        }
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err(Error::InvalidData);
        }
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Error::Transport)?,
            origin: url.as_str().trim_end_matches('/').into(),
            token,
        })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request::<(), T>(Method::GET, path, None).await
    }
    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        self.request(Method::POST, path, Some(body)).await
    }
    pub async fn post_empty<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.bytes(Method::POST, path, Some(body)).await.map(|_| ())
    }
    pub(crate) async fn bytes<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<Vec<u8>> {
        // Paths cannot redirect credentials to a different origin.
        if !path.starts_with('/') || path.starts_with("//") || path.contains('#') {
            return Err(Error::InvalidData);
        }
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.origin))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(b)
        }
        let response = req.send().await.map_err(|_| Error::Transport)?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|n| n > 32 * 1024 * 1024)
        {
            return Err(Error::InvalidData);
        }
        let mut stream = response.bytes_stream();
        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::Transport)?;
            if data.len() + chunk.len() > 32 * 1024 * 1024 {
                return Err(Error::InvalidData);
            }
            data.extend_from_slice(&chunk)
        }
        if !status.is_success() {
            return Err(Error::Http(
                status.as_u16(),
                serde_json::from_slice::<ErrorResponse>(&data)
                    .ok()
                    .map(|e| e.code),
            ));
        }
        Ok(data)
    }
    pub async fn request<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<T> {
        Ok(serde_json::from_slice(
            &self.bytes(method, path, body).await?,
        )?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_policy() {
        for origin in [
            "http://example.com",
            "https://u:p@example.com",
            "https://example.com/foo",
            "https://example.com?secret=yes",
            "ftp://example.com",
            "https://example.com#frag",
        ] {
            assert!(Api::new(origin, "secret".into()).is_err())
        }
        for origin in [
            "https://example.com",
            "http://127.0.0.1:19383",
            "http://[::1]:2323",
        ] {
            assert!(Api::new(origin, "secret".into()).is_ok())
        }
    }
    #[test]
    fn errors_do_not_contain_credentials() {
        let e = Error::Http(401, Some(ErrorCode::Unauthorized));
        assert_eq!(e.to_string(), "client error Http(401, Some(Unauthorized))");
    }
}
