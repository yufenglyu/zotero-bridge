// Prevent an extra console window from staying open on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Zotero Bridge desktop app (M4): system tray + settings window.
//!
//! All sync, index and file work reuses the shared crates; the frontend
//! only displays status and edits configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State};
use tracing::{info, warn};
use zotero_bridge_core::{Config, Platform};
use zotero_bridge_index::Database;
use zotero_bridge_sync::SyncEngine;
use zotero_bridge_zotero_api::LocalApiClient;

struct AppState {
    config_path: PathBuf,
    db_path: PathBuf,
    config: RwLock<Config>,
    paused: AtomicBool,
    last_sync_error: Mutex<Option<String>>,
}

// ---------------------------------------------------------------------
// IPC view types
// ---------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct LibraryView {
    kind: String,
    zotero_library_id: String,
    display_name: String,
    last_version: u64,
    enabled: bool,
    last_error: Option<String>,
}

#[derive(Serialize, Clone)]
struct MirrorDirectoryView {
    platform: String,
    enabled: bool,
    directory: String,
    extension: String,
    expected_files: usize,
    actual_files: usize,
    missing_files: usize,
    orphan_files: usize,
    stale_files: usize,
    latest_created_at: Option<String>,
    latest_modified_at: Option<String>,
}

#[derive(Serialize)]
struct StatusView {
    instance: String,
    item_count: u64,
    library_count: u64,
    pending_jobs: u64,
    failed_jobs: u64,
    last_sync_at: Option<String>,
    paused: bool,
    zotero_running: bool,
    libraries: Vec<LibraryView>,
    mirror_directories: Vec<MirrorDirectoryView>,
}

#[derive(Serialize)]
struct PathsView {
    config_dir: String,
}

#[derive(Deserialize)]
struct RestorePrefsOptions {
    #[serde(default)]
    restore_paths: bool,
    #[serde(default)]
    mappings: Vec<zotero_bridge_core::prefs_backup::PathMapping>,
}

// ---------------------------------------------------------------------
// Sync helpers
// ---------------------------------------------------------------------

/// `Send`-safe wrapper: rusqlite connections are `!Sync`, so the engine
/// runs on a dedicated blocking thread with its own current-thread runtime.
async fn sync_once_send(config: Config, db_path: PathBuf) -> zotero_bridge_core::Result<String> {
    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(zotero_bridge_core::Error::Io)?;
        rt.block_on(sync_once(&config, &db_path))
    })
    .await
    .map_err(|e| zotero_bridge_core::Error::Config(format!("sync worker join: {e}")))?
}

async fn sync_once(
    config: &Config,
    db_path: &std::path::Path,
) -> zotero_bridge_core::Result<String> {
    let client = LocalApiClient::new(
        &config.zotero.api_base,
        config.zotero.request_timeout_seconds,
    )?;
    let mut db = Database::open(db_path)?;
    let report = {
        let mut engine = SyncEngine::new(&client, &mut db, config);
        engine.sync_all(false).await?
    };

    let mut mirror_summary = String::new();
    for platform in zotero_bridge_sync::engine::enabled_platforms(config) {
        let mut total = zotero_bridge_mirror::worker::WorkerReport::default();
        for _ in 0..20 {
            let batch = zotero_bridge_mirror::worker::process_pending(&db, platform, 1000)?;
            total.completed += batch.completed;
            total.retried += batch.retried;
            total.failed += batch.failed;
            if batch.completed < 1000 {
                break;
            }
        }
        if total.completed + total.failed > 0 {
            mirror_summary = format!(
                "；镜像 {} 完成 {}{}",
                platform.as_str(),
                total.completed,
                if total.failed > 0 {
                    format!("，失败 {}", total.failed)
                } else {
                    String::new()
                }
            );
        }
    }

    let upserted: usize = report.libraries.iter().map(|l| l.upserted).sum();
    let deleted: usize = report.libraries.iter().map(|l| l.deleted).sum();
    Ok(format!(
        "同步完成：+{} 条，-{} 条{}",
        upserted, deleted, mirror_summary
    ))
}

// ---------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<StatusView, String> {
    let db = Database::open(&state.db_path).map_err(|e| e.to_string())?;
    let stats = db.stats().map_err(|e| e.to_string())?;
    let libraries = db
        .list_libraries(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|l| LibraryView {
            kind: l.remote.kind.as_str().to_string(),
            zotero_library_id: l.remote.zotero_library_id,
            display_name: l.remote.display_name,
            last_version: l.state.last_version,
            enabled: l.state.enabled,
            last_error: l.state.last_error,
        })
        .collect();
    let instance = db.active_instance_id().ok().flatten().unwrap_or_default();
    let cfg = state.config.read().unwrap().clone();
    let mirror_directories = zotero_bridge_sync::inspect_mirror_directories(&db, &cfg)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|m| MirrorDirectoryView {
            platform: m.platform.as_str().to_string(),
            enabled: m.enabled,
            directory: m.directory,
            extension: m.extension,
            expected_files: m.expected_files,
            actual_files: m.actual_files,
            missing_files: m.missing_files,
            orphan_files: m.orphan_files,
            stale_files: m.stale_files,
            latest_created_at: m.latest_created_at,
            latest_modified_at: m.latest_modified_at,
        })
        .collect();

    let zotero_running = {
        match LocalApiClient::new(&cfg.zotero.api_base, 2) {
            Ok(client) => {
                use zotero_bridge_zotero_api::ZoteroSource;
                client.probe().await.is_ok()
            }
            Err(_) => false,
        }
    };

    Ok(StatusView {
        instance,
        item_count: stats.item_count,
        library_count: stats.library_count,
        pending_jobs: stats.pending_jobs,
        failed_jobs: stats.failed_jobs,
        last_sync_at: stats.last_sync_at,
        paused: state.paused.load(Ordering::Relaxed),
        zotero_running,
        libraries,
        mirror_directories,
    })
}

#[tauri::command]
async fn sync_now(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = state.config.read().unwrap().clone();
    let db_path = state.db_path.clone();
    let result = sync_once_send(cfg, db_path).await;
    match &result {
        Ok(_) => *state.last_sync_error.lock().unwrap() = None,
        Err(e) => *state.last_sync_error.lock().unwrap() = Some(e.to_string()),
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn set_paused(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    state.paused.store(paused, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    Ok(state.config.read().unwrap().clone())
}

#[tauri::command]
fn get_paths(state: State<'_, AppState>) -> PathsView {
    let config_dir = state
        .config_path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    PathsView { config_dir }
}

#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> Result<String, String> {
    zotero_bridge_core::config::save(&state.config_path, &config).map_err(|e| e.to_string())?;
    apply_autostart(&app, config.app.start_at_login);
    *state.config.write().unwrap() = config;
    Ok("设置已保存。".to_string())
}

#[tauri::command]
fn rebuild_index(state: State<'_, AppState>) -> Result<String, String> {
    let db = Database::open(&state.db_path).map_err(|e| e.to_string())?;
    db.rebuild_fts().map_err(|e| e.to_string())?;
    let ok = db.fts_integrity_check().map_err(|e| e.to_string())?;
    Ok(format!(
        "FTS 索引已重建，一致性检查：{}",
        if ok { "OK" } else { "失败" }
    ))
}

#[tauri::command]
fn zotero_template() -> Option<String> {
    zotero_bridge_core::zotero_prefs::attachment_rename_template()
}

#[tauri::command]
fn scan_zotero_prefs(
    path: Option<String>,
) -> Result<zotero_bridge_core::prefs_backup::PrefsScan, String> {
    zotero_bridge_core::prefs_backup::scan_from_path(clean_path(path)).map_err(|e| e.to_string())
}

#[tauri::command]
fn backup_zotero_prefs(
    path: Option<String>,
    source_path: Option<String>,
    prefs: Option<Vec<zotero_bridge_core::prefs_backup::PrefEntry>>,
) -> Result<String, String> {
    let out = clean_path(path);
    let source = clean_path(source_path);
    let result = if let Some(prefs) = prefs {
        zotero_bridge_core::prefs_backup::backup_entries_from(source, out, prefs)
    } else {
        zotero_bridge_core::prefs_backup::backup_current_from(source, out)
    };
    result
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn load_zotero_prefs_backup(
    path: String,
) -> Result<zotero_bridge_core::prefs_backup::PrefsBackup, String> {
    zotero_bridge_core::prefs_backup::load_backup(PathBuf::from(path).as_path())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn preview_restore_zotero_prefs(
    path: String,
    options: RestorePrefsOptions,
    prefs: Option<Vec<zotero_bridge_core::prefs_backup::PrefEntry>>,
    target_path: Option<String>,
) -> Result<zotero_bridge_core::prefs_backup::RestorePreview, String> {
    let target = clean_path(target_path);
    let opts = zotero_bridge_core::prefs_backup::RestoreOptions {
        restore_paths: options.restore_paths,
        mappings: options.mappings,
    };
    if let Some(prefs) = prefs {
        zotero_bridge_core::prefs_backup::preview_restore_entries_to(
            target,
            "已编辑设置项",
            prefs,
            &opts,
        )
        .map_err(|e| e.to_string())
    } else {
        zotero_bridge_core::prefs_backup::preview_restore_to(
            target,
            PathBuf::from(path).as_path(),
            &opts,
        )
        .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn restore_zotero_prefs(
    path: String,
    options: RestorePrefsOptions,
    prefs: Option<Vec<zotero_bridge_core::prefs_backup::PrefEntry>>,
    target_path: Option<String>,
) -> Result<zotero_bridge_core::prefs_backup::RestoreReport, String> {
    if zotero_is_running() {
        return Err("检测到 Zotero 正在运行。请先关闭 Zotero，再应用恢复。".into());
    }
    let target = clean_path(target_path);
    let opts = zotero_bridge_core::prefs_backup::RestoreOptions {
        restore_paths: options.restore_paths,
        mappings: options.mappings,
    };
    if let Some(prefs) = prefs {
        zotero_bridge_core::prefs_backup::apply_restore_entries_to(target, prefs, &opts)
            .map_err(|e| e.to_string())
    } else {
        zotero_bridge_core::prefs_backup::apply_restore_to(
            target,
            PathBuf::from(path).as_path(),
            &opts,
        )
        .map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn save_zotero_prefs(
    path: Option<String>,
    prefs: Vec<zotero_bridge_core::prefs_backup::PrefEntry>,
) -> Result<zotero_bridge_core::prefs_backup::RestoreReport, String> {
    if zotero_is_running() {
        return Err("检测到 Zotero 正在运行。请先关闭 Zotero，再写回设置。".into());
    }
    zotero_bridge_core::prefs_backup::save_current_entries(clean_path(path), prefs)
        .map_err(|e| e.to_string())
}

fn clean_path(path: Option<String>) -> Option<PathBuf> {
    path.filter(|p| !p.trim().is_empty()).map(PathBuf::from)
}

#[cfg(windows)]
fn zotero_is_running() -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq zotero.exe"])
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .to_ascii_lowercase()
                .contains("zotero.exe")
        })
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn zotero_is_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "Zotero"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn zotero_is_running() -> bool {
    false
}

#[tauri::command]
fn refresh_links(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = state.config.read().unwrap().clone();
    let mut db = Database::open(&state.db_path).map_err(|e| e.to_string())?;
    let report = zotero_bridge_sync::refresh_mirrors(&mut db, &cfg).map_err(|e| e.to_string())?;
    let mut completed = 0usize;
    let mut failed = 0usize;
    for platform in zotero_bridge_sync::enabled_platforms(&cfg) {
        let w = zotero_bridge_mirror::worker::process_pending(&db, platform, 10000)
            .map_err(|e| e.to_string())?;
        completed += w.completed;
        failed += w.failed;
    }
    Ok(format!(
        "链接已刷新：改名 {}，补写/覆盖 {}，删除 {}，无需变动 {}{}",
        report.renamed,
        report.rewritten,
        report.deleted,
        report.unchanged,
        if failed > 0 {
            format!("（{} 项写入失败，可在诊断页排查）", failed)
        } else {
            format!("（写入 {} 项）", completed)
        }
    ))
}

#[tauri::command]
fn open_dir(state: State<'_, AppState>, which: String) -> Result<(), String> {
    let cfg = state.config.read().unwrap().clone();
    let dir = match which.as_str() {
        "mirror" => cfg.mirror_dir(Platform::current()),
        "config" => state
            .config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default(),
        "logs" => zotero_bridge_core::paths::log_dir().map_err(|e| e.to_string())?,
        other => return Err(format!("未知目录：{other}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_folder(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn browse_backup_file(default_dir: Option<String>) -> Option<String> {
    let mut dialog = rfd::FileDialog::new()
        .add_filter("Zotero Bridge 设置备份", &["json"])
        .set_file_name("zotero-bridge-prefs-backup.json");
    if let Some(dir) = dialog_start_dir(default_dir) {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn browse_restore_file(default_dir: Option<String>) -> Option<String> {
    let mut dialog = rfd::FileDialog::new().add_filter("Zotero Bridge 设置备份", &["json"]);
    if let Some(dir) = dialog_start_dir(default_dir) {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file().map(|p| p.to_string_lossy().into_owned())
}

fn dialog_start_dir(path: Option<String>) -> Option<PathBuf> {
    let path = clean_path(path)?;
    if path.is_dir() {
        Some(path)
    } else {
        path.parent().map(PathBuf::from)
    }
}

#[cfg(windows)]
fn open_folder(dir: &PathBuf) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    // explorer.exe returns 1 even on success; only spawn errors matter.
    std::process::Command::new("explorer.exe")
        .arg(dir)
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn open_folder(dir: &PathBuf) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg(dir)
        .spawn()
        .map(|_| ())
}

#[tauri::command]
async fn doctor(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let cfg = state.config.read().unwrap().clone();
    let mut lines = Vec::new();
    let push = |lines: &mut Vec<String>, ok: bool, label: String| {
        lines.push(format!("[{}] {}", if ok { "OK" } else { "FAIL" }, label));
    };

    match LocalApiClient::new(&cfg.zotero.api_base, cfg.zotero.request_timeout_seconds) {
        Ok(client) => {
            use zotero_bridge_zotero_api::ZoteroSource;
            match client.probe().await {
                Ok(info) => {
                    push(&mut lines, true, "Zotero process reachable".into());
                    push(&mut lines, true, "Local API enabled".into());
                    push(
                        &mut lines,
                        true,
                        format!("API version: {}", info.api_version.unwrap_or(0)),
                    );
                    push(
                        &mut lines,
                        !info.server_id.is_empty(),
                        if info.server_id.is_empty() {
                            "Server ID missing (legacy fallback active)".into()
                        } else {
                            "Server ID detected".into()
                        },
                    );
                }
                Err(zotero_bridge_core::Error::ApiDisabled) => {
                    push(&mut lines, true, "Zotero process reachable".into());
                    push(
                        &mut lines,
                        false,
                        "Local API disabled：Zotero 设置 → 高级 → 允许其他应用程序通信".into(),
                    );
                }
                Err(e) => push(&mut lines, false, format!("Zotero reachable ({e})")),
            }
        }
        Err(e) => push(&mut lines, false, format!("HTTP client init ({e})")),
    }

    let fts_ok = Database::open_in_memory()
        .and_then(|db| db.rebuild_fts().map(|_| true))
        .unwrap_or(false);
    push(&mut lines, fts_ok, "SQLite FTS5 available".into());

    for platform in zotero_bridge_sync::engine::enabled_platforms(&cfg) {
        let dir = cfg.mirror_dir(platform);
        let ok = std::fs::create_dir_all(&dir)
            .and_then(|_| {
                let probe = dir.join(".zotero-bridge-write-test");
                std::fs::write(&probe, b"ok")?;
                std::fs::remove_file(&probe)
            })
            .is_ok();
        push(
            &mut lines,
            ok,
            format!("Mirror directory writable: {}", dir.display()),
        );
    }

    match Database::open(&state.db_path) {
        Ok(db) => push(
            &mut lines,
            true,
            format!(
                "Index database ok ({} items)",
                db.stats().map(|s| s.item_count).unwrap_or(0)
            ),
        ),
        Err(e) => push(&mut lines, false, format!("Index database ({e})")),
    }

    Ok(lines)
}

// ---------------------------------------------------------------------
// Autostart / tray / background loop
// ---------------------------------------------------------------------

fn apply_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        warn!(error = %e, "autostart update failed");
    }
}

fn spawn_sync_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let paused;
            let cfg;
            let db_path;
            {
                let state = app.state::<AppState>();
                paused = state.paused.load(Ordering::Relaxed);
                cfg = state.config.read().unwrap().clone();
                db_path = state.db_path.clone();
            }
            if !paused {
                match sync_once_send(cfg, db_path).await {
                    Ok(summary) => {
                        info!("{summary}");
                        let state = app.state::<AppState>();
                        *state.last_sync_error.lock().unwrap() = None;
                    }
                    Err(zotero_bridge_core::Error::ZoteroOffline(_)) => {
                        // Quiet backoff: Zotero simply is not running.
                    }
                    Err(e) => {
                        warn!(error = %e, "background sync failed");
                        let state = app.state::<AppState>();
                        *state.last_sync_error.lock().unwrap() = Some(e.to_string());
                    }
                }
            }
            let interval = {
                let state = app.state::<AppState>();
                let seconds = state
                    .config
                    .read()
                    .unwrap()
                    .app
                    .poll_interval_seconds
                    .max(5);
                seconds
            };
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        }
    });
}

fn main() {
    let config_path = zotero_bridge_core::paths::resolve_config_file(None).expect("config path");
    let config = zotero_bridge_core::config::load_or_create(&config_path).expect("load config");
    let db_path = zotero_bridge_core::paths::resolve_database_file(
        None,
        Some(&config.storage.database),
        zotero_bridge_core::paths::is_portable_config(&config_path),
    )
    .expect("database path");

    // File logging into the platform log directory.
    if let Ok(log_dir) = zotero_bridge_core::paths::log_dir() {
        let _ = std::fs::create_dir_all(&log_dir);
        let appender = tracing_appender::rolling::never(&log_dir, "zotero-bridge.log");
        let (writer, _guard) = tracing_appender::non_blocking(appender);
        tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .init();
        // Keep the guard alive for the process lifetime.
        Box::leak(Box::new(_guard));
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState {
            config_path,
            db_path,
            config: RwLock::new(config),
            paused: AtomicBool::new(false),
            last_sync_error: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            sync_now,
            set_paused,
            get_config,
            get_paths,
            save_config,
            rebuild_index,
            refresh_links,
            zotero_template,
            scan_zotero_prefs,
            backup_zotero_prefs,
            load_zotero_prefs_backup,
            preview_restore_zotero_prefs,
            restore_zotero_prefs,
            save_zotero_prefs,
            open_dir,
            browse_backup_file,
            browse_restore_file,
            doctor,
        ])
        .setup(|app| {
            // Apply autostart preference.
            let state = app.state::<AppState>();
            let start_at_login = state.config.read().unwrap().app.start_at_login;
            apply_autostart(app.handle(), start_at_login);

            // System tray.
            let show = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
            let sync = MenuItemBuilder::with_id("sync", "立即同步").build(app)?;
            let pause = MenuItemBuilder::with_id("pause", "暂停同步").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show, &sync, &pause])
                .separator()
                .items(&[&quit])
                .build()?;

            let icon_bytes = include_bytes!("../icons/32x32.png");
            let icon = tauri::image::Image::from_bytes(icon_bytes).expect("tray icon png");

            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip("Zotero Bridge")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "sync" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let cfg;
                            let db_path;
                            {
                                let state = app.state::<AppState>();
                                cfg = state.config.read().unwrap().clone();
                                db_path = state.db_path.clone();
                            }
                            match sync_once_send(cfg, db_path).await {
                                Ok(s) => info!("tray sync: {s}"),
                                Err(e) => warn!("tray sync failed: {e}"),
                            }
                        });
                    }
                    "pause" => {
                        let state = app.state::<AppState>();
                        let now = !state.paused.load(Ordering::Relaxed);
                        state.paused.store(now, Ordering::Relaxed);
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Background incremental sync loop.
            spawn_sync_loop(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button hides to tray instead of quitting.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Zotero Bridge");
}
