//! HTTP client for the Zotero Local API.

use crate::dto::{DeletedResponse, VersionsResponse, ZoteroGroup, ZoteroItem};
use crate::source::{DeletedObjects, ItemResponse, VersionResponse, ZoteroSource};
use std::time::Duration;
use zsb_core::{Error, RemoteLibrary, Result, ServerInfo};

/// Maximum number of item keys per batch request (spec section 12.1).
pub const BATCH_SIZE: usize = 50;

/// Page size used when paginating `format=versions` responses.
const VERSIONS_PAGE: u32 = 100;

pub struct LocalApiClient {
    http: reqwest::Client,
    /// Base URL including the `/api` suffix, e.g. "http://localhost:23119/api".
    api_base: String,
}

impl LocalApiClient {
    pub fn new(api_base: &str, timeout_seconds: u64) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(1)))
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(LocalApiClient {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }

    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    /// Map an HTTP response to domain errors; returns the response when OK.
    async fn checked(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        match status.as_u16() {
            s if (200..300).contains(&s) => Ok(resp),
            403 => Err(Error::ApiDisabled),
            412 => Err(Error::InstanceChanged),
            s => {
                let body = resp.text().await.unwrap_or_default();
                Err(Error::Api {
                    status: s,
                    message: body.chars().take(200).collect(),
                })
            }
        }
    }

    fn map_reqwest_err(e: reqwest::Error) -> Error {
        if e.is_connect() || e.is_timeout() {
            Error::ZoteroOffline(e.to_string())
        } else {
            Error::Http(e.to_string())
        }
    }

    fn header_u64(resp: &reqwest::Response, name: &str) -> Option<u64> {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    }

    fn header_string(resp: &reqwest::Response, name: &str) -> Option<String> {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    async fn get_text(&self, url: &str, query: &[(&str, String)]) -> Result<(String, u64)> {
        let resp = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .map_err(Self::map_reqwest_err)?;
        let last_modified = Self::header_u64(&resp, "last-modified-version").unwrap_or(0);
        let resp = Self::checked(resp).await?;
        let body = resp.text().await.map_err(Self::map_reqwest_err)?;
        Ok((body, last_modified))
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<(T, u64)> {
        let (body, last_modified) = self.get_text(url, query).await?;
        let parsed: T = serde_json::from_str(&body).map_err(|e| {
            Error::Json(format!(
                "{e}; body starts: {}",
                &body[..body.len().min(120)]
            ))
        })?;
        Ok((parsed, last_modified))
    }
}

impl ZoteroSource for LocalApiClient {
    async fn probe(&self) -> Result<ServerInfo> {
        let resp = self
            .http
            .get(self.url("/"))
            .send()
            .await
            .map_err(Self::map_reqwest_err)?;
        let api_version = Self::header_u64(&resp, "zotero-api-version").map(|v| v as u32);
        let schema_version = Self::header_u64(&resp, "zotero-schema-version").map(|v| v as u32);
        let server_id = Self::header_string(&resp, "zotero-server-id").unwrap_or_default();
        let resp = Self::checked(resp).await?;
        drop(resp);
        Ok(ServerInfo {
            api_base: self.api_base.clone(),
            api_version,
            schema_version,
            server_id,
        })
    }

    async fn list_libraries(&self) -> Result<Vec<RemoteLibrary>> {
        let mut libs = vec![RemoteLibrary::user()];
        let (groups, _): (Vec<ZoteroGroup>, u64) =
            self.get_json(&self.url("/users/0/groups"), &[]).await?;
        for g in groups {
            libs.push(RemoteLibrary::group(g.id.to_string(), g.data.name));
        }
        Ok(libs)
    }

    async fn changed_item_versions(
        &self,
        library: &RemoteLibrary,
        since: u64,
    ) -> Result<VersionResponse> {
        let url = self.url(&format!("{}/items/top", library.api_prefix));
        let mut all = VersionsResponse::new();
        let mut start = 0u32;
        let mut last_modified = 0u64;
        loop {
            let (body, lm) = self
                .get_text(
                    &url,
                    &[
                        ("since", since.to_string()),
                        ("format", "versions".into()),
                        ("includeTrashed", "1".into()),
                        ("start", start.to_string()),
                        ("limit", VERSIONS_PAGE.to_string()),
                    ],
                )
                .await?;
            let page = crate::dto::parse_versions(&body)
                .map_err(|e| Error::Json(format!("versions response: {e}")))?;
            last_modified = last_modified.max(lm);
            let count = page.len() as u32;
            all.extend(page);
            if count < VERSIONS_PAGE {
                break;
            }
            start += VERSIONS_PAGE;
        }
        Ok(VersionResponse {
            versions: all,
            last_modified_version: last_modified,
        })
    }

    async fn fetch_items(&self, library: &RemoteLibrary, keys: &[String]) -> Result<ItemResponse> {
        if keys.is_empty() {
            return Ok(ItemResponse::default());
        }
        let url = self.url(&format!("{}/items", library.api_prefix));
        let (items, lm): (Vec<ZoteroItem>, u64) = self
            .get_json(
                &url,
                &[("itemKey", keys.join(",")), ("includeTrashed", "1".into())],
            )
            .await?;
        Ok(ItemResponse {
            items,
            last_modified_version: lm,
        })
    }

    async fn deleted_objects(&self, library: &RemoteLibrary, since: u64) -> Result<DeletedObjects> {
        let url = self.url(&format!("{}/deleted", library.api_prefix));
        let (deleted, lm): (DeletedResponse, u64) =
            self.get_json(&url, &[("since", since.to_string())]).await?;
        Ok(DeletedObjects {
            deleted,
            last_modified_version: lm,
        })
    }
}
