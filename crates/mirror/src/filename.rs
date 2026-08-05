//! Mirror filename templates and cross-platform sanitization
//! (spec sections 13.1 and 13.2).

use zsb_core::IndexedItem;

/// Maximum base filename length (without extension), per spec section 13.2.
pub const MAX_BASENAME_CHARS: usize = 180;

const ILLEGAL_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Render a template like `{primary_creator} - {year} - {title} -- {item_key}`
/// into a sanitized base filename (no directory, no extension).
///
/// The `{item_key}` placeholder is mandatory: it guarantees uniqueness and
/// makes stale-file cleanup reliable (spec section 13.1). If the template
/// lacks it, it is appended.
pub fn render(template: &str, item: &IndexedItem) -> String {
    let mut template = template.to_string();
    if !template.contains("{item_key}") {
        template.push_str(" -- {item_key}");
    }

    let raw = substitute(&template, item, &item.title);
    let mut name = sanitize(&raw);

    // Enforce the length budget by shrinking the title first, keeping
    // author + year + title-start + item key (spec section 13.2).
    if name.chars().count() > MAX_BASENAME_CHARS {
        let tail = format!(" -- {}", item.item_key);
        let head = substitute(&template, item, "");
        // `head` contains everything except the title (plus separators).
        let fixed = head.chars().count().max(tail.chars().count());
        let budget = MAX_BASENAME_CHARS.saturating_sub(fixed + 1);
        let short_title: String = item.title.chars().take(budget).collect();
        let short_title = short_title.trim_end().to_string();
        name = sanitize(&substitute(&template, item, &short_title));
    }

    if name.is_empty() {
        name = format!("untitled -- {}", item.item_key);
    }
    name
}

fn substitute(template: &str, item: &IndexedItem, title: &str) -> String {
    let mut out = template.to_string();
    let replacements: &[(&str, &str)] = &[
        ("{primary_creator}", &item.primary_creator),
        ("{creators}", &item.creators),
        ("{year}", &item.year),
        ("{title}", title),
        ("{container_title}", &item.container_title),
        ("{item_key}", &item.item_key),
        ("{item_type}", &item.item_type),
    ];
    for (placeholder, value) in replacements {
        out = out.replace(placeholder, value);
    }
    // Tidy up separators left behind by empty fields: " -  - " -> " - ".
    while out.contains(" -  - ") {
        out = out.replace(" -  - ", " - ");
    }
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim_matches(|c| c == ' ' || c == '-').to_string()
}

/// Render with automatic syntax detection: templates containing `{{` use
/// the Zotero filename template syntax (ztemplate.rs); anything else uses
/// the legacy `{placeholder}` syntax.
///
/// Unlike the legacy renderer, Zotero templates are NOT forced to include
/// `{item_key}`: uniqueness is handled by the sync engine's collision
/// fallback (appending ` -- <key>` only when two items would produce the
/// same filename).
pub fn render_auto(template: &str, item: &IndexedItem) -> String {
    if !crate::ztemplate::is_zotero_template(template) {
        return render(template, item);
    }
    let raw = crate::ztemplate::render(template, item);
    let mut name = sanitize(raw.trim());
    if name.chars().count() > MAX_BASENAME_CHARS {
        name = name.chars().take(MAX_BASENAME_CHARS).collect();
        name = name.trim_end_matches([' ', '.']).to_string();
    }
    if name.is_empty() {
        name = format!("untitled -- {}", item.item_key);
    }
    name
}

/// Append ` -- <item_key>` to a base filename, keeping the total within
/// the length budget. Used by the engine when a rendered name collides
/// with another item's.
pub fn with_key_suffix(base: &str, item_key: &str) -> String {
    let suffix = format!(" -- {item_key}");
    let budget = MAX_BASENAME_CHARS.saturating_sub(suffix.chars().count());
    let head: String = base.chars().take(budget).collect();
    format!("{}{}", head.trim_end_matches([' ', '.']), suffix)
}

/// Make a string safe as a filename on Windows and macOS.
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if ILLEGAL_CHARS.contains(&c) {
            out.push('_');
        } else if c.is_control() {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    // Windows forbids trailing spaces and dots.
    let mut cleaned = out.trim_end_matches([' ', '.']).to_string();
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    // Windows reserved device names (checked before the first dot).
    let stem = cleaned.split('.').next().unwrap_or("").to_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        cleaned = format!("_{cleaned}");
    }
    cleaned.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, creator: &str, year: &str, key: &str) -> IndexedItem {
        IndexedItem {
            title: title.into(),
            primary_creator: creator.into(),
            year: year.into(),
            item_key: key.into(),
            ..Default::default()
        }
    }

    #[test]
    fn default_template_rendering() {
        let i = item("燃气轮机转子动力学研究", "张三", "2024", "N49R8KAQ");
        assert_eq!(
            render("{primary_creator} - {year} - {title} -- {item_key}", &i),
            "张三 - 2024 - 燃气轮机转子动力学研究 -- N49R8KAQ"
        );
    }

    #[test]
    fn missing_year_collapses_separator() {
        let i = item("标题", "张三", "", "KEY12345");
        assert_eq!(
            render("{primary_creator} - {year} - {title} -- {item_key}", &i),
            "张三 - 标题 -- KEY12345"
        );
    }

    #[test]
    fn illegal_characters_replaced() {
        let i = item("a<b>c:d\"e/f\\g|h?i*j", "x", "2024", "K1");
        let name = render("{title} -- {item_key}", &i);
        assert!(!name.chars().any(|c| ILLEGAL_CHARS.contains(&c)));
        assert!(name.ends_with("-- K1"));
    }

    #[test]
    fn reserved_names_prefixed() {
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("con"), "_con");
        assert_eq!(sanitize("COM1"), "_COM1");
        assert_eq!(sanitize("LPT9"), "_LPT9");
        assert_eq!(sanitize("NORMAL"), "NORMAL");
    }

    #[test]
    fn trailing_dots_and_spaces_removed() {
        assert_eq!(sanitize("name.  "), "name");
        assert_eq!(sanitize("name ..."), "name");
    }

    #[test]
    fn long_titles_truncated_keep_key() {
        let long_title = "长".repeat(500);
        let i = item(&long_title, "张三", "2024", "N49R8KAQ");
        let name = render("{primary_creator} - {year} - {title} -- {item_key}", &i);
        assert!(name.chars().count() <= MAX_BASENAME_CHARS);
        assert!(name.ends_with("-- N49R8KAQ"));
        assert!(name.starts_with("张三 - 2024 - "));
    }

    #[test]
    fn missing_item_key_placeholder_is_appended() {
        let i = item("标题", "张三", "2024", "KEY12345");
        let name = render("{title}", &i);
        assert!(name.ends_with("-- KEY12345"));
    }
}
