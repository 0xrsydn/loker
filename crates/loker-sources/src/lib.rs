//! Source adapters. Each adapter fetches raw items and maps them to `NewJob`;
//! a shared `SourceAdapter` trait arrives with the second adapter.

pub mod himalayas;

/// HTTP client with loker's User-Agent: the tool and its version, no personal details.
pub fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("loker/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}
