//! Windows `.url` Internet Shortcut backend (spec section 13.3).

use crate::MirrorBackend;
use zsb_core::Platform;

pub struct WindowsUrlBackend;

impl MirrorBackend for WindowsUrlBackend {
    fn platform(&self) -> Platform {
        Platform::Windows
    }

    fn extension(&self) -> &'static str {
        "url"
    }

    fn build_content(&self, select_uri: &str) -> String {
        format!("[InternetShortcut]\r\nURL={select_uri}\r\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_file_content() {
        let content =
            WindowsUrlBackend.build_content("zotero://select/library/items/N49R8KAQ");
        assert_eq!(
            content,
            "[InternetShortcut]\r\nURL=zotero://select/library/items/N49R8KAQ\r\n"
        );
    }
}
