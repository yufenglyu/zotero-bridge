//! Structured backup and restore for Zotero `prefs.js`.
//!
//! The module intentionally avoids whole-file replacement. It exports
//! structured `user_pref` entries, classifies path-like and sensitive settings
//! across Zotero and plugin namespaces, previews changes, and merges selected
//! values back into the current profile after creating a `.bak` file.

use crate::zotero_prefs::default_profile_dir;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrefValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

impl PrefValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            PrefValue::String(s) => Some(s),
            _ => None,
        }
    }

    fn to_js_literal(&self) -> String {
        match self {
            PrefValue::Bool(v) => v.to_string(),
            PrefValue::Integer(v) => v.to_string(),
            PrefValue::Float(v) => {
                let mut s = v.to_string();
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    s.push_str(".0");
                }
                s
            }
            PrefValue::String(v) => format!("\"{}\"", escape_js_string(v)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefKind {
    Portable,
    Path,
    Sensitive,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefScope {
    Zotero,
    Plugin,
    Browser,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefEntry {
    pub key: String,
    pub value: PrefValue,
    pub kind: PrefKind,
    pub scope: PrefScope,
    pub plugin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedPref {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefsBackup {
    pub format_version: u32,
    pub app: String,
    pub created_at: String,
    pub profile: String,
    pub prefs: Vec<PrefEntry>,
    pub skipped: Vec<SkippedPref>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefsScan {
    pub profile: String,
    pub prefs_file: String,
    pub total: usize,
    pub exportable: usize,
    pub portable: usize,
    pub paths: usize,
    pub plugins: usize,
    pub sensitive: usize,
    pub groups: Vec<PrefGroup>,
    pub prefs: Vec<PrefEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefGroup {
    pub id: String,
    pub label: String,
    pub scope: PrefScope,
    pub plugin: Option<String>,
    pub total: usize,
    pub exportable: usize,
    pub paths: usize,
    pub sensitive: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMapping {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreOptions {
    #[serde(default)]
    pub restore_paths: bool,
    #[serde(default)]
    pub mappings: Vec<PathMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestorePreview {
    pub backup_path: String,
    pub backup_items: usize,
    pub will_add: usize,
    pub will_modify: usize,
    pub unchanged: usize,
    pub skipped_paths: usize,
    pub path_items: Vec<PrefEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreReport {
    pub applied: usize,
    pub added: usize,
    pub modified: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub skipped_paths: usize,
    pub backup_file: String,
}

#[derive(Debug, Clone)]
struct ParsedPref {
    entry: PrefEntry,
    line_index: usize,
}

pub fn scan_current() -> crate::Result<PrefsScan> {
    scan_from_path(None)
}

pub fn scan_from_path(path: Option<PathBuf>) -> crate::Result<PrefsScan> {
    let (profile, prefs_file) = resolve_prefs_file(path)?;
    let text = std::fs::read_to_string(&prefs_file)?;
    let parsed = parse_prefs(&text);
    let mut scan = PrefsScan {
        profile: profile.to_string_lossy().into_owned(),
        prefs_file: prefs_file.to_string_lossy().into_owned(),
        total: parsed.len(),
        exportable: 0,
        portable: 0,
        paths: 0,
        plugins: 0,
        sensitive: 0,
        groups: Vec::new(),
        prefs: Vec::new(),
    };
    let mut groups = BTreeMap::<String, PrefGroup>::new();
    for pref in parsed {
        match pref.entry.kind {
            PrefKind::Sensitive => {
                scan.sensitive += 1;
                scan.exportable += 1;
            }
            PrefKind::Path => {
                scan.paths += 1;
                scan.exportable += 1;
            }
            PrefKind::Portable => {
                scan.portable += 1;
                scan.exportable += 1;
            }
            PrefKind::Unknown => scan.exportable += 1,
        }
        if pref.entry.scope == PrefScope::Plugin {
            scan.plugins += 1;
        }
        let group = groups
            .entry(group_id(&pref.entry))
            .or_insert_with(|| group_for(&pref.entry));
        group.total += 1;
        group.exportable += 1;
        if pref.entry.kind == PrefKind::Path {
            group.paths += 1;
        }
        if pref.entry.kind == PrefKind::Sensitive {
            group.sensitive += 1;
        }
        scan.prefs.push(pref.entry);
    }
    scan.groups = groups.into_values().collect();
    Ok(scan)
}

pub fn backup_current(out: Option<PathBuf>) -> crate::Result<PathBuf> {
    backup_current_from(None, out)
}

pub fn backup_current_from(
    source: Option<PathBuf>,
    out: Option<PathBuf>,
) -> crate::Result<PathBuf> {
    let (profile, prefs_file) = resolve_prefs_file(source)?;
    let text = std::fs::read_to_string(&prefs_file)?;
    let parsed = parse_prefs(&text);
    let mut prefs = Vec::new();
    let skipped = Vec::new();
    for parsed_pref in parsed {
        prefs.push(parsed_pref.entry);
    }
    let backup = PrefsBackup {
        format_version: 1,
        app: "Zotero Bridge".into(),
        created_at: timestamp(),
        profile: profile.to_string_lossy().into_owned(),
        prefs,
        skipped,
    };
    write_backup(out, &profile, backup)
}

pub fn backup_entries(out: Option<PathBuf>, prefs: Vec<PrefEntry>) -> crate::Result<PathBuf> {
    backup_entries_from(None, out, prefs)
}

pub fn backup_entries_from(
    source: Option<PathBuf>,
    out: Option<PathBuf>,
    prefs: Vec<PrefEntry>,
) -> crate::Result<PathBuf> {
    let (profile, _) = resolve_prefs_file(source)?;
    let backup = PrefsBackup {
        format_version: 1,
        app: "Zotero Bridge".into(),
        created_at: timestamp(),
        profile: profile.to_string_lossy().into_owned(),
        prefs,
        skipped: Vec::new(),
    };
    write_backup(out, &profile, backup)
}

fn write_backup(
    out: Option<PathBuf>,
    profile: &Path,
    backup: PrefsBackup,
) -> crate::Result<PathBuf> {
    let out = out.unwrap_or_else(|| {
        profile.join(format!(
            "zotero-bridge-prefs-backup-{}.json",
            file_timestamp()
        ))
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(&backup)
        .map_err(|e| crate::Error::Config(format!("序列化备份失败：{e}")))?;
    std::fs::write(&out, data)?;
    Ok(out)
}

pub fn preview_restore(path: &Path, options: &RestoreOptions) -> crate::Result<RestorePreview> {
    let backup = read_backup(path)?;
    preview_restore_entries(path.to_string_lossy().as_ref(), backup.prefs, options)
}

pub fn preview_restore_to(
    target: Option<PathBuf>,
    path: &Path,
    options: &RestoreOptions,
) -> crate::Result<RestorePreview> {
    let backup = read_backup(path)?;
    preview_restore_entries_to(
        target,
        path.to_string_lossy().as_ref(),
        backup.prefs,
        options,
    )
}

pub fn preview_restore_entries(
    label: &str,
    prefs: Vec<PrefEntry>,
    options: &RestoreOptions,
) -> crate::Result<RestorePreview> {
    preview_restore_entries_to(None, label, prefs, options)
}

pub fn preview_restore_entries_to(
    target: Option<PathBuf>,
    label: &str,
    prefs: Vec<PrefEntry>,
    options: &RestoreOptions,
) -> crate::Result<RestorePreview> {
    let current = current_pref_map_from(target)?;
    let mut preview = RestorePreview {
        backup_path: label.to_string(),
        backup_items: prefs.len(),
        will_add: 0,
        will_modify: 0,
        unchanged: 0,
        skipped_paths: 0,
        path_items: Vec::new(),
    };
    for entry in prefs {
        if entry.kind == PrefKind::Path {
            preview.path_items.push(entry.clone());
            if !options.restore_paths {
                preview.skipped_paths += 1;
                continue;
            }
        }
        let mapped = apply_mappings(entry, &options.mappings);
        match current.get(&mapped.key) {
            None => preview.will_add += 1,
            Some(v) if v == &mapped.value => preview.unchanged += 1,
            Some(_) => preview.will_modify += 1,
        }
    }
    Ok(preview)
}

pub fn apply_restore(path: &Path, options: &RestoreOptions) -> crate::Result<RestoreReport> {
    let backup = read_backup(path)?;
    apply_restore_entries(backup.prefs, options)
}

pub fn apply_restore_to(
    target: Option<PathBuf>,
    path: &Path,
    options: &RestoreOptions,
) -> crate::Result<RestoreReport> {
    let backup = read_backup(path)?;
    apply_restore_entries_to(target, backup.prefs, options)
}

pub fn load_backup(path: &Path) -> crate::Result<PrefsBackup> {
    read_backup(path)
}

pub fn apply_restore_entries(
    prefs: Vec<PrefEntry>,
    options: &RestoreOptions,
) -> crate::Result<RestoreReport> {
    apply_restore_entries_to(None, prefs, options)
}

pub fn apply_restore_entries_to(
    target: Option<PathBuf>,
    prefs: Vec<PrefEntry>,
    options: &RestoreOptions,
) -> crate::Result<RestoreReport> {
    let (_, prefs_file) = resolve_prefs_file(target)?;
    let text = std::fs::read_to_string(&prefs_file)?;
    let parsed = parse_prefs(&text);
    let mut by_key: HashMap<String, ParsedPref> = parsed
        .into_iter()
        .map(|p| (p.entry.key.clone(), p))
        .collect();

    let backup_file = prefs_file.with_extension(format!("js.{}.bak", file_timestamp()));
    std::fs::copy(&prefs_file, &backup_file)?;

    let mut additions = BTreeMap::<String, PrefValue>::new();
    let mut replacements = HashMap::<usize, PrefEntry>::new();
    let mut report = RestoreReport {
        applied: 0,
        added: 0,
        modified: 0,
        unchanged: 0,
        deleted: 0,
        skipped_paths: 0,
        backup_file: backup_file.to_string_lossy().into_owned(),
    };

    for entry in prefs {
        if entry.kind == PrefKind::Path && !options.restore_paths {
            report.skipped_paths += 1;
            continue;
        }
        let mapped = apply_mappings(entry, &options.mappings);
        if let Some(current) = by_key.remove(&mapped.key) {
            if current.entry.value == mapped.value {
                report.unchanged += 1;
            } else {
                replacements.insert(current.line_index, mapped);
                report.modified += 1;
                report.applied += 1;
            }
        } else {
            additions.insert(mapped.key, mapped.value);
            report.added += 1;
            report.applied += 1;
        }
    }

    let mut lines: Vec<String> = text.lines().map(ToOwned::to_owned).collect();
    for (line_index, entry) in replacements {
        if let Some(line) = lines.get_mut(line_index) {
            *line = pref_line(&entry.key, &entry.value);
        }
    }
    if !additions.is_empty() {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("// Zotero Bridge restored preferences".into());
        for (key, value) in additions {
            lines.push(pref_line(&key, &value));
        }
    }
    let mut output = lines.join("\n");
    output.push('\n');
    std::fs::write(&prefs_file, output)?;
    Ok(report)
}

pub fn save_current_entries(
    target: Option<PathBuf>,
    prefs: Vec<PrefEntry>,
) -> crate::Result<RestoreReport> {
    let (_, prefs_file) = resolve_prefs_file(target)?;
    let text = std::fs::read_to_string(&prefs_file)?;
    let parsed = parse_prefs(&text);
    let desired: BTreeMap<String, PrefEntry> =
        prefs.into_iter().map(|p| (p.key.clone(), p)).collect();

    let backup_file = prefs_file.with_extension(format!("js.{}.bak", file_timestamp()));
    std::fs::copy(&prefs_file, &backup_file)?;

    let mut report = RestoreReport {
        applied: 0,
        added: 0,
        modified: 0,
        unchanged: 0,
        deleted: 0,
        skipped_paths: 0,
        backup_file: backup_file.to_string_lossy().into_owned(),
    };
    let mut seen = std::collections::BTreeSet::<String>::new();
    let mut remove_lines = std::collections::BTreeSet::<usize>::new();
    let mut replacements = HashMap::<usize, PrefEntry>::new();

    for current in parsed {
        if let Some(next) = desired.get(&current.entry.key) {
            seen.insert(next.key.clone());
            if current.entry.value == next.value {
                report.unchanged += 1;
            } else {
                replacements.insert(current.line_index, next.clone());
                report.modified += 1;
                report.applied += 1;
            }
        } else {
            remove_lines.insert(current.line_index);
            report.deleted += 1;
            report.applied += 1;
        }
    }

    let additions: BTreeMap<String, PrefValue> = desired
        .into_iter()
        .filter(|(key, _)| !seen.contains(key))
        .map(|(key, entry)| (key, entry.value))
        .collect();
    report.added = additions.len();
    report.applied += additions.len();

    let mut lines: Vec<String> = text.lines().map(ToOwned::to_owned).collect();
    for (line_index, entry) in replacements {
        if let Some(line) = lines.get_mut(line_index) {
            *line = pref_line(&entry.key, &entry.value);
        }
    }
    let mut kept: Vec<String> = lines
        .into_iter()
        .enumerate()
        .filter_map(|(i, line)| (!remove_lines.contains(&i)).then_some(line))
        .collect();
    if !additions.is_empty() {
        if kept.last().is_some_and(|l| !l.trim().is_empty()) {
            kept.push(String::new());
        }
        kept.push("// Zotero Bridge saved preferences".into());
        for (key, value) in additions {
            kept.push(pref_line(&key, &value));
        }
    }
    let mut output = kept.join("\n");
    output.push('\n');
    std::fs::write(&prefs_file, output)?;
    Ok(report)
}

fn current_pref_map_from(target: Option<PathBuf>) -> crate::Result<HashMap<String, PrefValue>> {
    let (_, prefs_file) = resolve_prefs_file(target)?;
    let text = std::fs::read_to_string(prefs_file)?;
    Ok(parse_prefs(&text)
        .into_iter()
        .map(|p| (p.entry.key, p.entry.value))
        .collect())
}

fn resolve_prefs_file(path: Option<PathBuf>) -> crate::Result<(PathBuf, PathBuf)> {
    let Some(path) = path else {
        let profile = default_profile_dir()
            .ok_or_else(|| crate::Error::Config("找不到 Zotero profile。".into()))?;
        return Ok((profile.clone(), profile.join("prefs.js")));
    };
    let prefs_file = if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("prefs.js"))
        || path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("js"))
    {
        path
    } else {
        path.join("prefs.js")
    };
    let profile = prefs_file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| crate::Error::Config("prefs.js 路径无效。".into()))?;
    Ok((profile, prefs_file))
}

fn read_backup(path: &Path) -> crate::Result<PrefsBackup> {
    let text = std::fs::read_to_string(path)?;
    let mut backup: PrefsBackup = serde_json::from_str(&text)
        .map_err(|e| crate::Error::Config(format!("读取备份失败：{e}")))?;
    for entry in backup.prefs.iter_mut() {
        classify(entry);
    }
    Ok(backup)
}

fn parse_prefs(text: &str) -> Vec<ParsedPref> {
    text.lines()
        .enumerate()
        .filter_map(|(line_index, line)| {
            parse_pref_line(line).map(|entry| ParsedPref { entry, line_index })
        })
        .collect()
}

fn parse_pref_line(line: &str) -> Option<PrefEntry> {
    let line = line.trim();
    let rest = line.strip_prefix("user_pref(")?;
    let (key, rest) = parse_js_string(rest.trim_start())?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(',')?.trim_start();
    let (value, _) = parse_value(rest)?;
    let mut entry = PrefEntry {
        key,
        value,
        kind: PrefKind::Unknown,
        scope: PrefScope::Unknown,
        plugin: None,
    };
    classify(&mut entry);
    Some(entry)
}

fn parse_value(s: &str) -> Option<(PrefValue, &str)> {
    let s = s.trim_start();
    if s.starts_with('"') {
        let (value, rest) = parse_js_string(s)?;
        return Some((PrefValue::String(value), rest));
    }
    for literal in ["true", "false"] {
        if let Some(rest) = s.strip_prefix(literal) {
            return Some((PrefValue::Bool(literal == "true"), rest));
        }
    }
    let end = s
        .find(|c: char| c == ')' || c == ';' || c.is_whitespace())
        .unwrap_or(s.len());
    let raw = &s[..end];
    if raw.contains('.') {
        raw.parse::<f64>()
            .ok()
            .map(|v| (PrefValue::Float(v), &s[end..]))
    } else {
        raw.parse::<i64>()
            .ok()
            .map(|v| (PrefValue::Integer(v), &s[end..]))
    }
}

fn parse_js_string(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut i = 0usize;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next()?;
        if c == '"' {
            return Some((out, &rest[1..]));
        }
        if c == '\\' {
            let esc = rest[1..].chars().next()?;
            let decoded = match esc {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            };
            out.push(decoded);
            i += 1 + esc.len_utf8();
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
    None
}

fn classify(entry: &mut PrefEntry) {
    let key = entry.key.to_ascii_lowercase();
    entry.scope = scope_for(&entry.key);
    entry.plugin = plugin_for(&entry.key);
    if is_sensitive_key(&key) {
        entry.kind = PrefKind::Sensitive;
    } else if is_path_pref(&key, &entry.value) {
        entry.kind = PrefKind::Path;
    } else {
        entry.kind = PrefKind::Portable;
    }
}

fn group_id(entry: &PrefEntry) -> String {
    match entry.scope {
        PrefScope::Plugin => format!("plugin:{}", entry.plugin.as_deref().unwrap_or("unknown")),
        PrefScope::Zotero => "zotero".into(),
        PrefScope::Browser => "browser".into(),
        PrefScope::Unknown => "unknown".into(),
    }
}

fn group_for(entry: &PrefEntry) -> PrefGroup {
    let (id, label) = match entry.scope {
        PrefScope::Plugin => {
            let plugin = entry.plugin.as_deref().unwrap_or("未知插件");
            (format!("plugin:{plugin}"), format!("插件：{plugin}"))
        }
        PrefScope::Zotero => ("zotero".into(), "Zotero 本身".into()),
        PrefScope::Browser => ("browser".into(), "Zotero 框架".into()),
        PrefScope::Unknown => ("unknown".into(), "未知来源".into()),
    };
    PrefGroup {
        id,
        label,
        scope: entry.scope.clone(),
        plugin: entry.plugin.clone(),
        total: 0,
        exportable: 0,
        paths: 0,
        sensitive: 0,
    }
}

fn scope_for(key: &str) -> PrefScope {
    let lower = key.to_ascii_lowercase();
    if lower.starts_with("extensions.zotero.") {
        if plugin_for(key).is_some() {
            PrefScope::Plugin
        } else {
            PrefScope::Zotero
        }
    } else if lower.starts_with("extensions.") {
        if plugin_for(key).is_some() {
            PrefScope::Plugin
        } else {
            PrefScope::Browser
        }
    } else if lower.starts_with("browser.") || lower.starts_with("app.") {
        PrefScope::Browser
    } else {
        PrefScope::Unknown
    }
}

fn plugin_for(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    let zotero_native = [
        "extensions.zotero.attachment",
        "extensions.zotero.annotations",
        "extensions.zotero.automatic",
        "extensions.zotero.autorenamefiles",
        "extensions.zotero.baseattachmentpath",
        "extensions.zotero.cite",
        "extensions.zotero.databaseschema",
        "extensions.zotero.debug",
        "extensions.zotero.duplicatelibraries",
        "extensions.zotero.downloadautomatically",
        "extensions.zotero.downloadassociatedfiles",
        "extensions.zotero.export",
        "extensions.zotero.extensions",
        "extensions.zotero.file",
        "extensions.zotero.first",
        "extensions.zotero.hiddennotices",
        "extensions.zotero.http",
        "extensions.zotero.httpserver",
        "extensions.zotero.ignorelegacydatadir",
        "extensions.zotero.import",
        "extensions.zotero.integration",
        "extensions.zotero.itempaneheader",
        "extensions.zotero.keys",
        "extensions.zotero.launch",
        "extensions.zotero.lastrenameassociatedfile",
        "extensions.zotero.lastselectedprefpane",
        "extensions.zotero.lastviewedfolder",
        "extensions.zotero.local",
        "extensions.zotero.newitemtypemru",
        "extensions.zotero.note",
        "extensions.zotero.pane",
        "extensions.zotero.panes",
        "extensions.zotero.postupgradebannerversionshown",
        "extensions.zotero.preferences",
        "extensions.zotero.prefversion",
        "extensions.zotero.reader",
        "extensions.zotero.recentsavetargets",
        "extensions.zotero.recursivecollections",
        "extensions.zotero.saverelativeattachmentpath",
        "extensions.zotero.secondarysort",
        "extensions.zotero.showattachmentfilenames",
        "extensions.zotero.sidebarstate",
        "extensions.zotero.sourcelist",
        "extensions.zotero.sync",
        "extensions.zotero.tabs",
        "extensions.zotero.tagselector",
        "extensions.zotero.translators",
        "extensions.zotero.usedatadir",
        "extensions.zotero.datadir",
        "extensions.zotero.warn",
    ];
    let framework_native = [
        "extensions.blocklist",
        "extensions.databaseschema",
        "extensions.lastappbuildid",
        "extensions.lastappversion",
        "extensions.lastplatformversion",
        "extensions.pendingoperations",
        "extensions.signaturecheckpoint",
        "extensions.systemaddonset",
        "extensions.ui",
        "extensions.webextensions",
        "extensions.zoteroopenofficeintegration",
        "extensions.zoterowinwordintegration",
    ];
    if lower.starts_with("extensions.zotero.") {
        if zotero_native.iter().any(|p| lower.starts_with(p)) {
            return None;
        }
        let mut parts = key.split('.');
        let _ = parts.next();
        let _ = parts.next();
        return parts.next().map(ToOwned::to_owned);
    }
    if lower.starts_with("extensions.") {
        if framework_native.iter().any(|p| lower.starts_with(p)) {
            return None;
        }
        let mut parts = key.split('.');
        let _ = parts.next();
        return parts.next().map(ToOwned::to_owned);
    }
    None
}

fn is_sensitive_key(key: &str) -> bool {
    let sensitive_terms = [
        "password",
        "passwd",
        "secret",
        "token",
        "accesstoken",
        "refreshtoken",
        "credential",
        "cookie",
        "session",
        "oauth",
        "username",
        "email",
        "account",
    ];
    sensitive_terms.iter().any(|term| key.contains(term))
        || ((key.contains("apikey") || key.contains("api_key")) && !key.contains("localapi"))
}

fn is_path_pref(_key: &str, value: &PrefValue) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    looks_like_path(trimmed)
}

fn looks_like_path(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("zotero://")
        || value.starts_with('{')
        || value.starts_with('[')
    {
        return false;
    }
    let b = value.as_bytes();
    (b.len() >= 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || value.starts_with("~/")
        || value.starts_with("/Users/")
        || value.starts_with("/Volumes/")
        || value.starts_with("/home/")
        || value.starts_with("/mnt/")
        || value.starts_with("/media/")
        || value.starts_with("/tmp/")
        || value.starts_with("file:///")
        || value.starts_with("\\\\")
}

fn apply_mappings(mut entry: PrefEntry, mappings: &[PathMapping]) -> PrefEntry {
    if entry.kind != PrefKind::Path {
        return entry;
    }
    let PrefValue::String(value) = &entry.value else {
        return entry;
    };
    let Some(mapped) = map_path_value(value, mappings) else {
        return entry;
    };
    entry.value = PrefValue::String(mapped);
    entry
}

fn map_path_value(value: &str, mappings: &[PathMapping]) -> Option<String> {
    for mapping in mappings {
        if mapping.from.trim().is_empty() {
            continue;
        }
        if starts_with_path_prefix(value, &mapping.from) {
            return Some(format!("{}{}", mapping.to, &value[mapping.from.len()..]));
        }
    }
    None
}

fn starts_with_path_prefix(value: &str, prefix: &str) -> bool {
    if value.starts_with(prefix) {
        return true;
    }
    value
        .to_ascii_lowercase()
        .starts_with(&prefix.to_ascii_lowercase())
}

fn pref_line(key: &str, value: &PrefValue) -> String {
    format!(
        "user_pref(\"{}\", {});",
        escape_js_string(key),
        value.to_js_literal()
    )
}

fn escape_js_string(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| match c {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            c => vec![c],
        })
        .collect()
}

fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

fn file_timestamp() -> String {
    time::OffsetDateTime::now_utc().unix_timestamp().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_classifies_prefs() {
        let text = r#"
user_pref("extensions.zotero.attachmentRenameTemplate", "{{title}}");
user_pref("extensions.zotero.somePlugin.outputPath", "D:\\Data\\Zotero");
user_pref("extensions.zotero.sync.server.username", "secret-user");
user_pref("browser.startup.page", 1);
"#;
        let prefs = parse_prefs(text);
        assert_eq!(prefs.len(), 4);
        assert_eq!(prefs[0].entry.kind, PrefKind::Portable);
        assert_eq!(prefs[1].entry.kind, PrefKind::Path);
        assert_eq!(prefs[1].entry.scope, PrefScope::Plugin);
        assert_eq!(prefs[1].entry.plugin.as_deref(), Some("somePlugin"));
        assert_eq!(prefs[2].entry.kind, PrefKind::Sensitive);
        assert_eq!(prefs[3].entry.value, PrefValue::Integer(1));
    }

    #[test]
    fn builds_pref_groups() {
        let text = r#"
user_pref("extensions.zotero.attachmentRenameTemplate", "{{title}}");
user_pref("extensions.zotero.somePlugin.outputPath", "D:\\Data\\Zotero");
user_pref("extensions.zotero.somePlugin.enabled", true);
user_pref("browser.startup.page", 1);
"#;
        let mut groups = BTreeMap::<String, PrefGroup>::new();
        for pref in parse_prefs(text) {
            let group = groups
                .entry(group_id(&pref.entry))
                .or_insert_with(|| group_for(&pref.entry));
            group.total += 1;
            if pref.entry.kind != PrefKind::Sensitive {
                group.exportable += 1;
            }
            if pref.entry.kind == PrefKind::Path {
                group.paths += 1;
            }
        }
        assert_eq!(groups.get("zotero").unwrap().total, 1);
        assert_eq!(groups.get("plugin:somePlugin").unwrap().total, 2);
        assert_eq!(groups.get("plugin:somePlugin").unwrap().paths, 1);
        assert_eq!(groups.get("browser").unwrap().total, 1);
    }

    #[test]
    fn path_mapping_updates_prefix() {
        let entry = PrefEntry {
            key: "extensions.x.path".into(),
            value: PrefValue::String("D:\\Data\\Docs\\Zotero".into()),
            kind: PrefKind::Path,
            scope: PrefScope::Plugin,
            plugin: Some("x".into()),
        };
        let mapped = apply_mappings(
            entry,
            &[PathMapping {
                from: "D:\\Data".into(),
                to: "E:\\Archive".into(),
            }],
        );
        assert_eq!(
            mapped.value,
            PrefValue::String("E:\\Archive\\Docs\\Zotero".into())
        );
    }

    #[test]
    fn writes_js_pref_line_with_escaped_string() {
        assert_eq!(
            pref_line(
                "extensions.x.template",
                &PrefValue::String("a\n\"b\"".into())
            ),
            "user_pref(\"extensions.x.template\", \"a\\n\\\"b\\\"\");"
        );
    }

    #[test]
    fn parses_reference_zotero_prefs_file() {
        let prefs = parse_prefs(include_str!("../../../assets/prefs.js"));
        assert!(prefs.len() >= 40);
        assert!(prefs.iter().any(|p| p.entry.scope == PrefScope::Zotero));
        assert!(!prefs.iter().any(|p| p.entry.scope == PrefScope::Plugin));
        assert!(prefs.iter().any(|p| p.entry.kind == PrefKind::Path));
    }

    #[test]
    fn classifies_common_zotero_native_prefixes() {
        let text = r#"
user_pref("extensions.zotero.annotations.noteTemplates.title", "x");
user_pref("extensions.zotero.export.quickCopy.setting", "x");
user_pref("extensions.zotero.import.charset", "utf-8");
user_pref("extensions.zotero.databaseSchema", 44);
user_pref("extensions.zotero.keys.copySelectedItemsToClipboard", "x");
user_pref("extensions.zotero.sourceList.persist", "x");
user_pref("extensions.zotero.tabs.selectedID", "x");
user_pref("extensions.zotero.tagSelector.sortMode", "x");
user_pref("extensions.zotero.automaticTags", true);
user_pref("extensions.zotero.automaticSnapshots", true);
user_pref("extensions.zotero.saveRelativeAttachmentPath", true);
user_pref("extensions.zotero.httpServer.enabled", true);
user_pref("extensions.zotero.recentSaveTargets", "[]");
user_pref("extensions.zotero.newItemTypeMRU", "book");
user_pref("extensions.zotero.postUpgradeBannerVersionShown", 1);
user_pref("extensions.zotero.lastRenameAssociatedFile", 1);
user_pref("extensions.zotero.lastSelectedPrefPane", "advanced");
user_pref("extensions.zoteroWinWordIntegration.version", "1");
"#;
        let prefs = parse_prefs(text);
        assert!(prefs
            .iter()
            .all(|p| p.entry.scope == PrefScope::Zotero || p.entry.scope == PrefScope::Browser));
        assert!(!prefs.iter().any(|p| p.entry.scope == PrefScope::Plugin));
    }

    #[test]
    fn classifies_zotero_ten_framework_prefixes_as_non_plugins() {
        let text = r#"
user_pref("extensions.pendingOperations", false);
user_pref("extensions.systemAddonSet", "{}");
user_pref("extensions.ui.dictionary.hidden", true);
user_pref("extensions.webextensions.uuids", "{}");
user_pref("extensions.zotero.secondarySort.title", 1);
user_pref("extensions.zotero.showAttachmentFilenames", true);
user_pref("extensions.zotero.night.enabled", true);
"#;
        let prefs = parse_prefs(text);
        for pref in prefs.iter().filter(|p| p.entry.key.contains("night")) {
            assert_eq!(pref.entry.scope, PrefScope::Plugin);
            assert_eq!(pref.entry.plugin.as_deref(), Some("night"));
        }
        for pref in prefs.iter().filter(|p| !p.entry.key.contains("night")) {
            assert_ne!(pref.entry.scope, PrefScope::Plugin, "{}", pref.entry.key);
            assert!(pref.entry.plugin.is_none(), "{}", pref.entry.key);
        }
    }

    #[test]
    fn path_detection_rejects_non_path_values_with_path_like_keys() {
        let text = r#"
user_pref("extensions.zotero.export.bibliographySettings", "{\"mode\":\"bibliography\",\"method\":\"copy-to-clipboard\"}");
user_pref("extensions.zotero.export.lastLocale", "en-US");
user_pref("extensions.zotero.export.lastStyle", "http://www.zotero.org/styles/elsevier-harvard");
user_pref("extensions.zotero.export.quickCopy.setting", "bibliography=http://www.zotero.org/styles/apa");
user_pref("extensions.zotero.lastViewedFolder", "L1");
user_pref("extensions.zotero.secondarySort.archiveLocation", "itemType");
user_pref("extensions.zotero.zoteroattanger.filetypes", "pdf,djvu,epub,doc,docx,ppt,pptx,xls,xlsx");
user_pref("extensions.zotero.zoteroattanger.dest_dir", "D:\\OneDrive\\Obsidian\\附件\\Zotero\\PDF");
"#;
        let prefs = parse_prefs(text);
        let paths: Vec<_> = prefs
            .iter()
            .filter(|p| p.entry.kind == PrefKind::Path)
            .map(|p| p.entry.key.as_str())
            .collect();
        assert_eq!(paths, vec!["extensions.zotero.zoteroattanger.dest_dir"]);
    }

    #[test]
    fn loading_backup_reclassifies_stale_path_kinds() {
        let root = unique_test_dir("prefs-reclassify-backup");
        std::fs::create_dir_all(&root).unwrap();
        let backup_file = root.join("backup.json");
        std::fs::write(
            &backup_file,
            r#"{
  "format_version": 1,
  "app": "Zotero Bridge",
  "created_at": "test",
  "profile": "test",
  "prefs": [
    {
      "key": "extensions.zotero.export.lastLocale",
      "value": "en-US",
      "kind": "path",
      "scope": "zotero",
      "plugin": null
    },
    {
      "key": "extensions.zotero.zoteroattanger.dest_dir",
      "value": "D:\\OneDrive\\Zotero\\PDF",
      "kind": "portable",
      "scope": "plugin",
      "plugin": "zoteroattanger"
    }
  ],
  "skipped": []
}"#,
        )
        .unwrap();

        let backup = load_backup(&backup_file).unwrap();
        assert_eq!(backup.prefs[0].kind, PrefKind::Portable);
        assert_eq!(backup.prefs[1].kind, PrefKind::Path);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resolves_explicit_prefs_file_or_profile_dir() {
        let root = unique_test_dir("prefs-path");
        let profile = root.join("Profiles").join("a.Test");
        std::fs::create_dir_all(&profile).unwrap();
        let prefs = profile.join("prefs.js");
        std::fs::write(&prefs, "").unwrap();

        let (from_dir, from_dir_file) = resolve_prefs_file(Some(profile.clone())).unwrap();
        assert_eq!(from_dir, profile);
        assert_eq!(from_dir_file, prefs);

        let (from_file, from_file_path) = resolve_prefs_file(Some(prefs.clone())).unwrap();
        assert_eq!(from_file, profile);
        assert_eq!(from_file_path, prefs);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saving_current_entries_updates_deletes_and_keeps_sensitive_in_scope() {
        let root = unique_test_dir("prefs-save");
        std::fs::create_dir_all(&root).unwrap();
        let prefs_file = root.join("prefs.js");
        std::fs::write(
            &prefs_file,
            r#"user_pref("extensions.zotero.keys.copySelectedItemsToClipboard", "old");
user_pref("extensions.zotero.somePlugin.enabled", true);
user_pref("extensions.zotero.sync.server.username", "secret-user");
"#,
        )
        .unwrap();

        let report = save_current_entries(
            Some(prefs_file.clone()),
            vec![
                PrefEntry {
                    key: "extensions.zotero.keys.copySelectedItemsToClipboard".into(),
                    value: PrefValue::String("new".into()),
                    kind: PrefKind::Portable,
                    scope: PrefScope::Zotero,
                    plugin: None,
                },
                PrefEntry {
                    key: "extensions.zotero.sync.server.username".into(),
                    value: PrefValue::String("changed-user".into()),
                    kind: PrefKind::Sensitive,
                    scope: PrefScope::Zotero,
                    plugin: None,
                },
            ],
        )
        .unwrap();
        let saved = std::fs::read_to_string(&prefs_file).unwrap();
        assert_eq!(report.modified, 2);
        assert_eq!(report.deleted, 1);
        assert!(saved.contains("copySelectedItemsToClipboard\", \"new\""));
        assert!(!saved.contains("somePlugin.enabled"));
        assert!(saved.contains("sync.server.username\", \"changed-user\""));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scan_and_backup_include_sensitive_prefs() {
        let root = unique_test_dir("prefs-sensitive");
        std::fs::create_dir_all(&root).unwrap();
        let prefs_file = root.join("prefs.js");
        std::fs::write(
            &prefs_file,
            r#"user_pref("extensions.zotero.sync.server.username", "secret-user");
user_pref("extensions.zotero.keys.copySelectedItemsToClipboard", "x");
"#,
        )
        .unwrap();
        let backup_file = root.join("backup.json");

        let scan = scan_from_path(Some(prefs_file.clone())).unwrap();
        assert_eq!(scan.total, 2);
        assert_eq!(scan.exportable, 2);
        assert_eq!(scan.sensitive, 1);
        assert!(scan
            .prefs
            .iter()
            .any(|p| p.key == "extensions.zotero.sync.server.username"));

        let out = backup_current_from(Some(prefs_file), Some(backup_file)).unwrap();
        let backup = load_backup(&out).unwrap();
        assert_eq!(backup.prefs.len(), 2);
        assert!(backup.skipped.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "zotero-bridge-{name}-{}",
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ))
    }
}
