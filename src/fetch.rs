//! Fetching Google API responses, through the cache when one is enabled.
//!
//! Every request goes through the [`Fetch`] trait rather than a client this
//! module builds itself. That is what lets a test observe which requests a
//! render actually makes, which is the only way to check that a cached run makes
//! none and that the map is probed before any Street View frame is paid for.

use std::future::Future;

use crate::cache::{Cache, Kind};

/// A request that did not produce bytes.
#[derive(Debug, Clone)]
pub struct FetchError {
    /// The HTTP status, when the request reached Google and came back refused.
    /// `None` means the request never completed.
    pub status: Option<u16>,
    pub message: String,
}

/// Something that can fetch the bytes at a URL.
pub trait Fetch: Sync {
    fn get(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, FetchError>> + Send;
}

/// A [`Fetch`] backed by a real HTTP client.
pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> HttpFetcher {
        HttpFetcher {
            client: reqwest::Client::new(),
        }
    }
}

impl Fetch for HttpFetcher {
    async fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let fail = |status, message: String| FetchError { status, message };
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| fail(None, e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(fail(
                Some(status.as_u16()),
                format!("request refused with {status}"),
            ));
        }
        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| fail(Some(status.as_u16()), e.to_string()))
    }
}

/// Fetch `url`, answering from `cache` when it can and storing whatever it had
/// to go and get.
pub async fn fetch_cached<F: Fetch>(
    fetcher: &F,
    cache: &Cache,
    kind: Kind,
    url: &str,
) -> Result<Vec<u8>, FetchError> {
    if let Some(hit) = cache.get(kind, url) {
        return Ok(hit);
    }
    let bytes = fetcher.get(url).await?;
    cache.put(kind, url, &bytes);
    Ok(bytes)
}
