//! zsb-launcher: open `zotero://select/...` URIs with the OS handler
//! (spec section 17.4). Zotero registers the `zotero://` scheme; the OS
//! starts Zotero if needed and focuses the item.

pub mod platform;

use zsb_core::{Error, Result};

/// Abstraction over the OS "open this URI" facility.
pub trait UriLauncher {
    fn open(&self, uri: &str) -> Result<()>;
}

/// Default system launcher.
///
/// Windows note: `explorer.exe <uri>` returns exit code 1 even on
/// success, so we use `rundll32 url.dll,FileProtocolHandler` instead.
/// macOS uses the `open` command via the `open` crate.
pub struct SystemLauncher;

impl UriLauncher for SystemLauncher {
    fn open(&self, uri: &str) -> Result<()> {
        if !platform::is_valid_zotero_uri(uri) {
            return Err(Error::Launcher(format!(
                "refusing to open non-zotero URI: {uri}"
            )));
        }
        open_impl(uri)
    }
}

#[cfg(windows)]
fn open_impl(uri: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", uri])
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Launcher(e.to_string()))
}

#[cfg(not(windows))]
fn open_impl(uri: &str) -> Result<()> {
    open::that(uri).map_err(|e| Error::Launcher(e.to_string()))
}
