//! Platform-specific launcher details.
//!
//! Windows: ShellExecute via `open` handles `zotero://` and `.url` files.
//! macOS: `open` handles `zotero://` and `.webloc` files; LaunchAgent
//! autostart is configured by the desktop app (M4), not here.

/// Whether the `zotero://` scheme appears usable on this machine.
/// The real check happens when Zotero is installed; we simply verify the
/// URI shape before handing it to the OS.
pub fn is_valid_zotero_uri(uri: &str) -> bool {
    uri.starts_with("zotero://select/") || uri.starts_with("zotero://open-pdf/")
}
