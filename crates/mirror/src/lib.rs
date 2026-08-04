//! zsb-mirror: filesystem mirror manager (spec section 13).
//!
//! Generates platform shortcut files that point at `zotero://select/...`
//! URIs, and executes persisted mirror jobs (outbox pattern, spec 8.4).

pub mod filename;
pub mod macos_webloc;
pub mod windows_url;
pub mod worker;

use zsb_core::Platform;

/// A platform-specific shortcut backend (spec section 17.3).
pub trait MirrorBackend: Send + Sync {
    fn platform(&self) -> Platform;
    fn extension(&self) -> &'static str;
    fn build_content(&self, select_uri: &str) -> String;
}

/// Get the backend for a platform.
pub fn backend_for(platform: Platform) -> &'static dyn MirrorBackend {
    match platform {
        Platform::Windows => &windows_url::WindowsUrlBackend,
        Platform::Macos => &macos_webloc::MacosWeblocBackend,
    }
}
