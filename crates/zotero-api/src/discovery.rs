//! Instance discovery helpers.
//!
//! Zotero 10+ sends a `Zotero-Server-ID` header identifying the current
//! database instance. Older versions do not; for them we fall back to a
//! locally persisted `legacy:<uuid>` identifier so caches stay isolated
//! per installation (spec section 4.1 / 8.1).

use zotero_bridge_core::{Result, ServerInfo};

use crate::source::ZoteroSource;

/// Probe the running Zotero and resolve a stable server id.
///
/// `legacy_id` is called only when the server does not send
/// `Zotero-Server-ID`; it must return a per-installation stable id
/// (already persisted by the caller).
pub async fn probe_instance<S: ZoteroSource>(
    source: &S,
    legacy_id: impl FnOnce() -> String,
) -> Result<ServerInfo> {
    let mut info = source.probe().await?;
    if info.server_id.is_empty() {
        info.server_id = format!("legacy:{}", legacy_id());
    }
    Ok(info)
}
