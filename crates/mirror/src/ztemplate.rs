//! Zotero 7 filename template syntax (`{{...}}`), as used by Zotero's own
//! attachment renaming. Supported constructs:
//!
//! - `{{if cond}}...{{elseif cond}}...{{else}}...{{endif}}` (nestable)
//! - conditions: `field == "value"`, `field != "value"`, or bare `field`
//!   (truthy = non-empty)
//! - field expressions with attributes:
//!   `{{authors max="1" initialize="given"}}`
//!   `{{date replaceFrom="[^0-9].*" replaceTo="" regexOpts="g"}}`
//!
//! Field values come from the item's preserved `raw_json` (the complete
//! Zotero `data` object), with fallbacks to the normalized IndexedItem
//! columns when raw JSON is unavailable.

use zsb_core::IndexedItem;

/// Whether a template uses Zotero `{{...}}` syntax (vs the legacy
/// `{placeholder}` syntax).
pub fn is_zotero_template(template: &str) -> bool {
    template.contains("{{")
}

/// Render a Zotero-syntax template to a raw (unsanitized) filename base.
pub fn render(template: &str, item: &IndexedItem) -> String {
    let data: serde_json::Value = item
        .raw_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .map(|v: serde_json::Value| v.get("data").cloned().unwrap_or(serde_json::Value::Null))
        .unwrap_or(serde_json::Value::Null);
    let ctx = Ctx { item, data: &data };
    let (nodes, _) = parse_nodes(template, &[]);
    let mut out = String::new();
    render_nodes(&nodes, &ctx, &mut out);
    // Collapse whitespace runs created by empty conditionals.
    let mut collapsed = out.trim().to_string();
    while collapsed.contains("  ") {
        collapsed = collapsed.replace("  ", " ");
    }
    collapsed
}

// ---------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------

enum Node {
    Text(String),
    Field(FieldExpr),
    If(Vec<Branch>),
}

struct Branch {
    cond: Option<Cond>, // None = else
    body: Vec<Node>,
}

struct FieldExpr {
    name: String,
    attrs: Vec<(String, String)>,
}

enum Cond {
    Truthy(String),
    Eq {
        field: String,
        value: String,
        negated: bool,
    },
}

/// Parse nodes until one of `stop_tags` ("elseif", "else", "endif") or the
/// end of input; returns the nodes and the stop tag encountered.
fn parse_nodes(s: &str, stop_tags: &[&str]) -> (Vec<Node>, Option<String>) {
    let mut nodes = Vec::new();
    let mut rest = s;
    loop {
        let Some(open) = rest.find("{{") else {
            if !rest.is_empty() {
                nodes.push(Node::Text(rest.to_string()));
            }
            return (nodes, None);
        };
        if open > 0 {
            nodes.push(Node::Text(rest[..open].to_string()));
        }
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            // Unterminated block: treat the remainder as text.
            nodes.push(Node::Text(rest[open..].to_string()));
            return (nodes, None);
        };
        let inner = after[..close].trim();
        rest = &after[close + 2..];

        let head = inner.split_whitespace().next().unwrap_or("");
        if stop_tags.contains(&head) {
            return (nodes, Some(head.to_string()));
        }
        match head {
            "if" => {
                let cond = parse_cond(inner[2..].trim());
                let mut branches = Vec::new();
                // scan_block finds the body extent (respecting nested ifs);
                // then the body slice is parsed recursively.
                let (consumed, next_stop) = scan_block(rest);
                let (body, _) = parse_nodes(&rest[..consumed], &[]);
                branches.push(Branch {
                    cond: Some(cond),
                    body,
                });
                let mut tail = &rest[consumed..];
                let mut stop = next_stop;
                loop {
                    match stop.as_deref() {
                        Some("elseif") => {
                            let tag_end = tail.find("}}").map(|i| i + 2).unwrap_or(0);
                            let cond_str = tail[2..tag_end - 2].trim()[6..].trim();
                            let c = parse_cond(cond_str);
                            tail = &tail[tag_end..];
                            let (used, ns) = scan_block(tail);
                            let (b, _) = parse_nodes(&tail[..used], &[]);
                            branches.push(Branch {
                                cond: Some(c),
                                body: b,
                            });
                            tail = &tail[used..];
                            stop = ns;
                        }
                        Some("else") => {
                            let tag_end = tail.find("}}").map(|i| i + 2).unwrap_or(0);
                            tail = &tail[tag_end..];
                            let (used, ns) = scan_block(tail);
                            let (b, _) = parse_nodes(&tail[..used], &[]);
                            branches.push(Branch {
                                cond: None,
                                body: b,
                            });
                            tail = &tail[used..];
                            stop = ns;
                        }
                        _ => {
                            // endif (or unterminated): consume the tag.
                            if stop.as_deref() == Some("endif") {
                                let tag_end = tail.find("}}").map(|i| i + 2).unwrap_or(0);
                                tail = &tail[tag_end..];
                            }
                            rest = tail;
                            break;
                        }
                    }
                }
                nodes.push(Node::If(branches));
            }
            "elseif" | "else" | "endif" => {
                // Stray closer without an opener: ignore it.
            }
            _ => {
                nodes.push(Node::Field(parse_field(inner)));
            }
        }
    }
}

/// Scan from the start of an if-body to its matching
/// elseif/else/endif (respecting nested ifs). Returns the byte length of
/// the body and which stop tag follows it.
fn scan_block(s: &str) -> (usize, Option<String>) {
    let mut depth = 0usize;
    let mut rest = s;
    let mut consumed = 0usize;
    loop {
        let Some(open) = rest.find("{{") else {
            return (s.len(), None);
        };
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return (s.len(), None);
        };
        let inner = after[..close].trim();
        let head = inner.split_whitespace().next().unwrap_or("");
        match head {
            "if" => depth += 1,
            "endif" => {
                if depth == 0 {
                    return (consumed + open, Some("endif".into()));
                }
                depth -= 1;
            }
            "elseif" | "else" => {
                if depth == 0 {
                    return (consumed + open, Some(head.into()));
                }
            }
            _ => {}
        }
        let adv = open + 2 + close + 2;
        consumed += adv;
        rest = &rest[adv..];
    }
}

fn parse_cond(s: &str) -> Cond {
    let parts = if let Some((l, r)) = s.split_once("!=") {
        Some((true, l, r))
    } else {
        s.split_once("==").map(|(l, r)| (false, l, r))
    };
    match parts {
        Some((negated, field, value)) => Cond::Eq {
            field: field.trim().to_string(),
            value: unquote(value.trim()),
            negated,
        },
        None => Cond::Truthy(s.trim().to_string()),
    }
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Parse `name attr="v" attr2='v2'` into a FieldExpr.
fn parse_field(inner: &str) -> FieldExpr {
    let mut chars = inner.char_indices().peekable();
    let mut name = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            break;
        }
        name.push(c);
        chars.next();
    }
    let mut attrs = Vec::new();
    let rest: String = chars.map(|(_, c)| c).collect();
    let mut r = rest.trim();
    while !r.is_empty() {
        let Some(eq) = r.find('=') else { break };
        let key = r[..eq].trim().to_string();
        r = r[eq + 1..].trim_start();
        let Some(&quote) = r.as_bytes().first() else {
            break;
        };
        if quote != b'"' && quote != b'\'' {
            break;
        }
        let q = quote as char;
        let body = &r[1..];
        let Some(end) = body.find(q) else { break };
        attrs.push((key, body[..end].to_string()));
        r = body[end + 1..].trim_start();
    }
    FieldExpr { name, attrs }
}

// ---------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------

struct Ctx<'a> {
    item: &'a IndexedItem,
    data: &'a serde_json::Value,
}

fn render_nodes(nodes: &[Node], ctx: &Ctx, out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Field(f) => out.push_str(&eval_field(f, ctx)),
            Node::If(branches) => {
                for b in branches {
                    let take = match &b.cond {
                        None => true,
                        Some(c) => eval_cond(c, ctx),
                    };
                    if take {
                        render_nodes(&b.body, ctx, out);
                        break;
                    }
                }
            }
        }
    }
}

fn eval_cond(cond: &Cond, ctx: &Ctx) -> bool {
    match cond {
        Cond::Truthy(field) => !resolve(field, ctx).trim().is_empty(),
        Cond::Eq {
            field,
            value,
            negated,
        } => {
            let eq = resolve(field, ctx).trim() == value;
            eq != *negated
        }
    }
}

fn eval_field(f: &FieldExpr, ctx: &Ctx) -> String {
    let mut value = match f.name.as_str() {
        "authors" => format_creators(ctx, "author", &f.attrs),
        "editors" => format_creators(ctx, "editor", &f.attrs),
        other => resolve(other, ctx),
    };
    // replaceFrom/replaceTo (regex), regexOpts "g" = replace all.
    let from = attr(&f.attrs, "replaceFrom");
    if let Some(from) = from {
        let to = attr(&f.attrs, "replaceTo").unwrap_or("");
        let global = attr(&f.attrs, "regexOpts")
            .map(|o| o.contains('g'))
            .unwrap_or(false);
        value = regex_replace(&value, from, to, global);
    }
    value
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn regex_replace(input: &str, from: &str, to: &str, global: bool) -> String {
    match regex::Regex::new(from) {
        Ok(re) => {
            if global {
                re.replace_all(input, to).into_owned()
            } else {
                re.replacen(input, 1, to).into_owned()
            }
        }
        Err(_) => {
            // Invalid regex: treat as literal text.
            if global {
                input.replace(from, to)
            } else {
                input.replacen(from, to, 1)
            }
        }
    }
}

/// Resolve a bare field name to its string value.
fn resolve(name: &str, ctx: &Ctx) -> String {
    match name {
        "title" => data_str(ctx, &["title"]).unwrap_or_else(|| ctx.item.title.clone()),
        "itemType" => data_str(ctx, &["itemType"]).unwrap_or_else(|| ctx.item.item_type.clone()),
        "date" => data_str(ctx, &["date", "issueDate", "filingDate"])
            .unwrap_or_else(|| ctx.item.year.clone()),
        "dateEnacted" => data_str(ctx, &["dateEnacted", "enactmentDate", "date"])
            .unwrap_or_else(|| ctx.item.year.clone()),
        "year" => {
            let date = resolve("date", ctx);
            if date.is_empty() {
                ctx.item.year.clone()
            } else {
                date.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            }
        }
        "itemKey" | "item_key" => ctx.item.item_key.clone(),
        "creators" => ctx.item.creators.clone(),
        "primaryCreator" | "primary_creator" => ctx.item.primary_creator.clone(),
        "nameOfAct" => data_str(ctx, &["nameOfAct", "title"]).unwrap_or_else(|| {
            if ctx.item.title.starts_with("[无标题] -- ") {
                String::new()
            } else {
                ctx.item.title.clone()
            }
        }),
        "publicLawNumber" => {
            data_str(ctx, &["publicLawNumber", "codeNumber", "number", "code"]).unwrap_or_default()
        }
        "containerTitle" | "container_title" | "publicationTitle" => {
            data_str(ctx, &["publicationTitle", "bookTitle", "proceedingsTitle"])
                .unwrap_or_else(|| ctx.item.container_title.clone())
        }
        "authors" => format_creators(ctx, "author", &[]),
        "editors" => format_creators(ctx, "editor", &[]),
        "number" => data_str(
            ctx,
            &["number", "reportNumber", "patentNumber", "caseNumber"],
        )
        .unwrap_or_default(),
        "publisher" => {
            data_str(ctx, &["publisher", "university", "institution"]).unwrap_or_default()
        }
        other => data_str(ctx, &[other]).unwrap_or_default(),
    }
}

/// First non-empty string among candidate keys in the item's data object.
fn data_str(ctx: &Ctx, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = ctx.data.get(k) {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => continue,
            };
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Format creators of one type, following Zotero 7's documented semantics
/// (https://www.zotero.org/support/file_renaming):
///
/// - `name` (default "family"): which name parts to include —
///   "family" (surname only), "given", "family-given", "given-family".
/// - `initialize` ("given" | "family" | "full"): convert the matching
///   *included* part(s) to initials. With the default name="family",
///   initialize="given" has no effect because the given part is excluded —
///   so `{{authors max="1" initialize="given"}}` yields just the surname.
/// - `initialize-with` (default "."): appended to each initial.
/// - `name-part-separator` (default " "): joins family/given parts.
/// - `join` (default ", "): joins consecutive creators.
/// - `max`: limit number of creators used.
///
/// Single-field (institutional) names always render verbatim.
fn format_creators(ctx: &Ctx, creator_type: &str, attrs: &[(String, String)]) -> String {
    let name_fmt = attr(attrs, "name").unwrap_or("family");
    let initialize = attr(attrs, "initialize");
    let init_with = attr(attrs, "initialize-with").unwrap_or(".");
    let part_sep = attr(attrs, "name-part-separator").unwrap_or(" ");
    let join = attr(attrs, "join").unwrap_or(", ");
    let max: Option<usize> = attr(attrs, "max").and_then(|m| m.parse().ok());

    let mut names: Vec<String> = Vec::new();
    if let Some(list) = ctx.data.get("creators").and_then(|c| c.as_array()) {
        for c in list {
            if c.get("creatorType").and_then(|t| t.as_str()) != Some(creator_type) {
                continue;
            }
            let single = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let last = c.get("lastName").and_then(|v| v.as_str()).unwrap_or("");
            let first = c.get("firstName").and_then(|v| v.as_str()).unwrap_or("");
            let name = if !single.trim().is_empty() {
                single.trim().to_string()
            } else {
                let family = maybe_initialize(last.trim(), "family", initialize, init_with);
                let given = maybe_initialize(first.trim(), "given", initialize, init_with);
                let parts: Vec<&str> = match name_fmt {
                    "given" => vec![given.as_str()],
                    "family-given" => vec![family.as_str(), given.as_str()],
                    "given-family" => vec![given.as_str(), family.as_str()],
                    // "family" and any unknown value: surname only.
                    _ => vec![family.as_str()],
                };
                parts
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(part_sep)
            };
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    if names.is_empty() && creator_type == "author" {
        // Fallback: normalized single primary creator.
        if !ctx.item.primary_creator.is_empty() {
            names.push(ctx.item.primary_creator.clone());
        }
    }
    if let Some(m) = max {
        names.truncate(m);
    }
    names.join(join)
}

/// Convert a name part to initials when `initialize` targets it
/// ("full" targets both parts). Each whitespace-separated word contributes
/// its first character plus `init_with`.
fn maybe_initialize(
    part: &str,
    part_kind: &str,
    initialize: Option<&str>,
    init_with: &str,
) -> String {
    let targeted = matches!(initialize, Some("full")) || initialize == Some(part_kind);
    if !targeted || part.is_empty() {
        return part.to_string();
    }
    part.split_whitespace()
        .filter_map(|w| w.chars().next().map(|c| format!("{c}{init_with}")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_with_raw(raw: &str, key: &str) -> IndexedItem {
        IndexedItem {
            item_key: key.into(),
            title: "标题".into(),
            item_type: "journalArticle".into(),
            primary_creator: "张三".into(),
            year: "2024".into(),
            raw_json: Some(raw.into()),
            ..Default::default()
        }
    }

    const RAW_ARTICLE: &str = r#"{"key":"K1","version":1,"data":{
        "itemType":"journalArticle",
        "title":"燃气轮机: 转子/动力学?",
        "date":"2024-03-01",
        "creators":[{"creatorType":"author","firstName":"Wei","lastName":"Wang"}]
    }}"#;

    #[test]
    fn detects_syntax() {
        assert!(is_zotero_template("{{title}}"));
        assert!(!is_zotero_template("{title} -- {item_key}"));
    }

    #[test]
    fn simple_fields_and_conditionals() {
        let i = item_with_raw(RAW_ARTICLE, "K1");
        let t = "{{if itemType == \"journalArticle\"}}J:{{else}}X:{{endif}}{{title}}";
        assert_eq!(render(t, &i), "J:燃气轮机: 转子/动力学?");
    }

    #[test]
    fn elseif_chain_falls_through() {
        let i = item_with_raw(
            r#"{"key":"K2","version":1,"data":{"itemType":"book","title":"T","creators":[],"publisher":"机械工业出版社","edition":"2"}}"#,
            "K2",
        );
        let t = "{{if itemType == \"journalArticle\"}}A{{elseif itemType == \"book\"}}《{{title}}》（{{if edition}}{{edition}}，{{endif}}{{publisher}}）{{else}}E{{endif}}";
        assert_eq!(render(t, &i), "《T》（2，机械工业出版社）");
    }

    #[test]
    fn authors_max_and_initialize() {
        let i = item_with_raw(RAW_ARTICLE, "K1");
        // Default name="family" excludes the given part entirely, so
        // initialize="given" has no effect: surname only (Zotero behavior).
        assert_eq!(
            render("{{authors max=\"1\" initialize=\"given\"}}", &i),
            "Wang"
        );
    }

    #[test]
    fn authors_name_and_initialize_variants() {
        let i = item_with_raw(RAW_ARTICLE, "K1");
        assert_eq!(
            render("{{authors name=\"given-family\" initialize=\"given\"}}", &i),
            "W. Wang"
        );
        assert_eq!(render("{{authors name=\"family-given\"}}", &i), "Wang Wei");
        assert_eq!(
            render(
                "{{authors name=\"given-family\" initialize=\"full\" initialize-with=\"\" name-part-separator=\"\"}}",
                &i
            ),
            "WW"
        );
    }

    #[test]
    fn authors_single_field_verbatim() {
        let i = item_with_raw(
            r#"{"key":"K6","version":1,"data":{"itemType":"book","title":"T","creators":[{"creatorType":"author","name":"张三"}]}}"#,
            "K6",
        );
        assert_eq!(
            render("{{authors max=\"1\" initialize=\"given\"}}", &i),
            "张三"
        );
    }

    #[test]
    fn authors_join_attr() {
        let i = item_with_raw(
            r#"{"key":"K7","version":1,"data":{"itemType":"journalArticle","title":"T","creators":[{"creatorType":"author","firstName":"Wei","lastName":"Wang"},{"creatorType":"author","firstName":"San","lastName":"Zhang"}]}}"#,
            "K7",
        );
        assert_eq!(render("{{authors}}", &i), "Wang, Zhang");
        assert_eq!(render("{{authors join=\" & \"}}", &i), "Wang & Zhang");
    }

    #[test]
    fn date_year_regex() {
        let i = item_with_raw(RAW_ARTICLE, "K1");
        let t = "{{date replaceFrom=\"[^0-9].*\" replaceTo=\"\" regexOpts=\"g\"}}";
        assert_eq!(render(t, &i), "2024");
    }

    #[test]
    fn illegal_char_regex() {
        let i = item_with_raw(RAW_ARTICLE, "K1");
        let t = "{{title replaceFrom='[\\\\/:?*\"<>|]' replaceTo=\"_\" regexOpts=\"g\"}}";
        assert_eq!(render(t, &i), "燃气轮机_ 转子_动力学_");
    }

    #[test]
    fn bare_field_truthiness() {
        let i = item_with_raw(
            r#"{"key":"K3","version":1,"data":{"itemType":"thesis","title":"T","creators":[]}}"#,
            "K3",
        );
        assert_eq!(
            render("{{if publisher}}（{{publisher}}）{{endif}}x", &i),
            "x"
        );
    }

    #[test]
    fn fallback_without_raw_json() {
        let mut i = item_with_raw("{}", "K4");
        i.raw_json = None;
        let t = "【{{authors}}-{{date}}】{{title}}";
        assert_eq!(render(t, &i), "【张三-2024】标题");
    }

    #[test]
    fn nested_conditionals() {
        let i = item_with_raw(
            r#"{"key":"K5","version":1,"data":{"itemType":"document","title":"T","creators":[],"versionNumber":"3"}}"#,
            "K5",
        );
        let t = "{{if itemType == \"document\"}}D{{if versionNumber}}（v{{versionNumber}}）{{endif}}{{else}}E{{endif}}";
        assert_eq!(render(t, &i), "D（v3）");
    }

    #[test]
    fn base_field_mappings() {
        // thesis: 模板里的 publisher 取 type-specific 的 university 字段
        let thesis = item_with_raw(
            r#"{"key":"K8","version":1,"data":{"itemType":"thesis","title":"T","date":"2016","university":"南京航空航天大学","creators":[{"creatorType":"author","name":"王铁成"}]}}"#,
            "K8",
        );
        assert_eq!(
            render(
                "【{{authors max=\"1\" initialize=\"given\"}}-{{date}}】{{title}}{{if publisher}}（{{publisher}}）{{endif}}",
                &thesis
            ),
            "【王铁成-2016】T（南京航空航天大学）"
        );
        // patent: 模板里的 date 取 issueDate（而非 filingDate）
        let patent = item_with_raw(
            r#"{"key":"K9","version":1,"data":{"itemType":"patent","title":"T","number":"CN113568705B","filingDate":"2021-07-23","issueDate":"2024-03-22","creators":[]}}"#,
            "K9",
        );
        assert_eq!(
            render(
                "{{number}} {{title}}{{if date}}（{{date replaceFrom=\"[^0-9].*\" replaceTo=\"\" regexOpts=\"g\"}}）{{endif}}",
                &patent
            ),
            "CN113568705B T（2024）"
        );
        // statute: Zotero legal records may use type-specific fields. Keep the
        // user's Zotero template working even when some fields are absent.
        let statute = item_with_raw(
            r#"{"key":"K10","version":1,"data":{"itemType":"statute","title":"中华人民共和国民法典","date":"2020-05-28","codeNumber":"主席令第四十五号","creators":[]}}"#,
            "K10",
        );
        assert_eq!(
            render(
                "《{{nameOfAct replaceFrom='[\\\\/:?*\"<>|]' replaceTo=\"_\" regexOpts=\"g\"}}》（{{if publicLawNumber}}{{publicLawNumber}}，{{endif}}{{if dateEnacted}}{{dateEnacted replaceFrom=\"[^0-9].*\" replaceTo=\"\" regexOpts=\"g\"}}{{endif}}）",
                &statute
            ),
            "《中华人民共和国民法典》（主席令第四十五号，2020）"
        );
    }

    #[test]
    fn user_template_smoke() {
        let t = "{{if itemType == \"journalArticle\"}}\n【{{authors max=\"1\" initialize=\"given\"}}{{if date}}-{{date replaceFrom=\"[^0-9].*\" replaceTo=\"\" regexOpts=\"g\"}}{{endif}}】{{title}}\n{{else}}\nother\n{{endif}}";
        let i = item_with_raw(RAW_ARTICLE, "K1");
        assert_eq!(render(t, &i), "【Wang-2024】燃气轮机: 转子/动力学?");
    }
}
