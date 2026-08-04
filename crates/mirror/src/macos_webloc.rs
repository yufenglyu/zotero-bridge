//! macOS `.webloc` backend (spec section 13.4).

use crate::MirrorBackend;
use zsb_core::Platform;

pub struct MacosWeblocBackend;

impl MirrorBackend for MacosWeblocBackend {
    fn platform(&self) -> Platform {
        Platform::Macos
    }

    fn extension(&self) -> &'static str {
        "webloc"
    }

    fn build_content(&self, select_uri: &str) -> String {
        // Escape the few characters that matter inside a plist string.
        let escaped = select_uri
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\"\n  \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n  \
             <key>URL</key>\n  \
             <string>{escaped}</string>\n\
             </dict>\n\
             </plist>\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webloc_content_contains_uri() {
        let content =
            MacosWeblocBackend.build_content("zotero://select/groups/123/items/ABC");
        assert!(content.contains("<string>zotero://select/groups/123/items/ABC</string>"));
        assert!(content.starts_with("<?xml"));
    }
}
