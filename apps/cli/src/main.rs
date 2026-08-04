//! zsb: Zotero Search Bridge command line interface (spec section 16).

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;
use zsb_core::{Config, Error, Platform, Result, SearchResult};
use zsb_index::Database;
use zsb_launcher::{SystemLauncher, UriLauncher};
use zsb_sync::SyncEngine;
use zsb_zotero_api::LocalApiClient;

#[derive(Parser)]
#[command(
    name = "zsb",
    version,
    about = "Zotero Search Bridge - external fast search and item locator for Zotero"
)]
struct Cli {
    /// Path to config.toml (default: platform config directory).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Path to the index database (default: platform data directory).
    #[arg(long, global = true)]
    database: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Plain,
    Json,
    Alfred,
}

#[derive(Subcommand)]
enum Commands {
    /// Search the local index. Works while Zotero is closed.
    Search {
        /// Keywords, e.g. "燃气轮机 转子" or author:Smith year:2024.
        query: String,
        #[arg(long, value_enum, default_value = "plain")]
        format: OutputFormat,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Open an item in Zotero via its select URI.
    Open {
        /// Library: "user" or "group:<id>".
        #[arg(long)]
        library: String,
        /// Zotero item key, e.g. N49R8KAQ.
        #[arg(long)]
        key: String,
    },
    /// Open an arbitrary zotero:// URI.
    OpenUri { uri: String },
    /// Synchronize the local index with the running Zotero.
    Sync {
        /// Re-read every item from version 0.
        #[arg(long)]
        full: bool,
        /// Sync only one library: "user" or "group:<id>".
        #[arg(long)]
        library: Option<String>,
        /// Keep running and sync every poll_interval_seconds.
        #[arg(long)]
        watch: bool,
    },
    /// Show index and sync status.
    Status,
    /// Run environment diagnostics.
    Doctor,
    /// Rebuild the FTS search index.
    Rebuild,
    /// Optimize the FTS index.
    Optimize,
    /// Verify FTS index consistency.
    VerifyIndex,
    /// Delete mirror files that no longer match any indexed item.
    CleanMirrors,
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::ZoteroOffline(msg)) => {
            eprintln!("Zotero 未运行或不可达：{msg}");
            eprintln!("提示：本地搜索仍然可用 (zsb search <关键词>)。");
            ExitCode::from(2)
        }
        Err(Error::ApiDisabled) => {
            eprintln!("Zotero Local API 未启用。请在 Zotero 中打开：");
            eprintln!("设置 → 高级 → 允许此计算机上的其他应用程序与 Zotero 通信");
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("错误：{e}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

async fn run(cli: Cli) -> Result<()> {
    let config_path = zsb_core::paths::resolve_config_file(cli.config.as_deref())?;
    let config = zsb_core::config::load_or_create(&config_path)?;
    let db_path = zsb_core::paths::resolve_database_file(
        cli.database.as_deref(),
        Some(&config.storage.database),
        zsb_core::paths::is_portable_config(&config_path),
    )?;

    match cli.command {
        Commands::Search {
            query,
            format,
            limit,
        } => {
            let db = Database::open(&db_path)?;
            let limit = limit
                .unwrap_or(config.search.default_limit)
                .clamp(1, config.search.maximum_limit);
            let q = zsb_index::build_search_query(&query, limit);
            let results = zsb_index::search::search(db.connection(), &q)?;
            print_results(&results, format);
        }
        Commands::Open { library, key } => {
            let (kind, id) = parse_library(&library)?;
            let db = Database::open(&db_path)?;
            let uri = db.find_select_uri(kind, &id, &key)?.ok_or_else(|| {
                Error::ItemNotFound(format!("{library}/{key}（先运行 zsb sync 建立索引）"))
            })?;
            SystemLauncher.open(&uri)?;
            println!("已打开：{uri}");
        }
        Commands::OpenUri { uri } => {
            SystemLauncher.open(&uri)?;
            println!("已打开：{uri}");
        }
        Commands::Sync {
            full,
            library,
            watch,
        } => {
            sync_command(&config, &db_path, full, library.as_deref(), watch).await?;
        }
        Commands::Status => {
            let db = Database::open(&db_path)?;
            print_status(&db)?;
        }
        Commands::Doctor => {
            doctor(&config, &db_path).await;
        }
        Commands::Rebuild => {
            let db = Database::open(&db_path)?;
            db.rebuild_fts()?;
            let ok = db.fts_integrity_check()?;
            println!(
                "FTS 索引已重建。一致性检查：{}",
                if ok { "OK" } else { "失败" }
            );
        }
        Commands::Optimize => {
            let db = Database::open(&db_path)?;
            db.optimize_fts()?;
            println!("FTS 索引已优化。");
        }
        Commands::VerifyIndex => {
            let db = Database::open(&db_path)?;
            let ok = db.fts_integrity_check()?;
            println!("FTS 一致性检查：{}", if ok { "OK" } else { "失败" });
            if !ok {
                println!("建议运行：zsb rebuild");
            }
        }
        Commands::CleanMirrors => {
            let db = Database::open(&db_path)?;
            clean_mirrors(&db, &config)?;
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// search output
// ----------------------------------------------------------------------

fn print_results(results: &[SearchResult], format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            if results.is_empty() {
                println!("没有匹配的结果。");
                return;
            }
            for (i, r) in results.iter().enumerate() {
                println!("{}. {}", i + 1, r.title);
                let mut line2 = String::new();
                if !r.creators.is_empty() {
                    line2.push_str(&r.creators);
                }
                if !r.year.is_empty() {
                    if !line2.is_empty() {
                        line2.push_str(" · ");
                    }
                    line2.push_str(&r.year);
                }
                if !r.container_title.is_empty() {
                    if !line2.is_empty() {
                        line2.push_str(" · ");
                    }
                    line2.push_str(&r.container_title);
                }
                if !line2.is_empty() {
                    println!("   {line2}");
                }
                println!("   {}", r.select_uri);
            }
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(results).unwrap_or_else(|_| "[]".into());
            println!("{json}");
        }
        OutputFormat::Alfred => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let mut subtitle_parts = Vec::new();
                    if !r.creators.is_empty() {
                        subtitle_parts.push(r.creators.clone());
                    }
                    if !r.year.is_empty() {
                        subtitle_parts.push(r.year.clone());
                    }
                    if !r.container_title.is_empty() {
                        subtitle_parts.push(r.container_title.clone());
                    }
                    serde_json::json!({
                        "uid": r.uid(),
                        "title": r.title,
                        "subtitle": subtitle_parts.join(" · "),
                        "arg": r.select_uri,
                        "valid": true,
                    })
                })
                .collect();
            let json = serde_json::to_string(&serde_json::json!({ "items": items }))
                .unwrap_or_else(|_| "{\"items\":[]}".into());
            println!("{json}");
        }
    }
}

// ----------------------------------------------------------------------
// sync
// ----------------------------------------------------------------------

async fn sync_command(
    config: &Config,
    db_path: &std::path::Path,
    full: bool,
    only_library: Option<&str>,
    watch: bool,
) -> Result<()> {
    loop {
        match sync_once(config, db_path, full, only_library).await {
            Ok(report) => {
                if report.libraries.is_empty() {
                    println!("没有需要同步的文献库。");
                }
                for lib in &report.libraries {
                    println!(
                        "[{}] {}: +{} 条, -{} 条, 镜像任务 {}, 版本 {} → {}",
                        lib.library_kind,
                        lib.display_name,
                        lib.upserted,
                        lib.deleted,
                        lib.mirror_jobs,
                        lib.from_version,
                        lib.to_version
                    );
                }
            }
            Err(Error::ZoteroOffline(_)) if watch => {
                eprintln!("等待 Zotero 启动…");
            }
            Err(e) => return Err(e),
        }

        if !watch {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(
            config.app.poll_interval_seconds.max(5),
        ))
        .await;
    }
}

async fn sync_once(
    config: &Config,
    db_path: &std::path::Path,
    full: bool,
    only_library: Option<&str>,
) -> Result<zsb_sync::SyncReport> {
    let client = LocalApiClient::new(
        &config.zotero.api_base,
        config.zotero.request_timeout_seconds,
    )?;
    let mut db = Database::open(db_path)?;
    let report = if let Some(sel) = only_library {
        let (kind, id) = parse_library(sel)?;
        let kind_enum = zsb_core::LibraryKind::parse(kind)
            .ok_or_else(|| Error::Config(format!("未知库类型：{kind}")))?;
        let remote = lookup_remote(&client, kind_enum, &id).await?;
        let info = zsb_zotero_api::discovery::probe_instance(&client, || "unpersisted".to_string())
            .await?;
        // The libraries row has a FK to zotero_instances; register the
        // instance before syncing a single library.
        db.upsert_instance(&info)?;
        let mut engine = SyncEngine::new(&client, &mut db, config);
        let mut report = zsb_sync::SyncReport {
            server_id: info.server_id.clone(),
            ..Default::default()
        };
        if let Some(lib_report) = engine.sync_library(&info.server_id, &remote, full).await? {
            report.libraries.push(lib_report);
        }
        report
    } else {
        let mut engine = SyncEngine::new(&client, &mut db, config);
        engine.sync_all(full).await?
    };

    // Process mirror jobs for every enabled platform (spec section 13),
    // draining the queue in batches until nothing is left.
    for platform in enabled_platforms(config) {
        let mut total = zsb_mirror::worker::WorkerReport::default();
        for _ in 0..20 {
            let batch = zsb_mirror::worker::process_pending(&db, platform, 1000)?;
            total.completed += batch.completed;
            total.retried += batch.retried;
            total.failed += batch.failed;
            if batch.completed == 0 || batch.completed < 1000 {
                break;
            }
        }
        if total.completed + total.retried + total.failed > 0 {
            println!(
                "镜像 [{}]: 完成 {}, 重试 {}, 失败 {}",
                platform.as_str(),
                total.completed,
                total.retried,
                total.failed
            );
        }
    }
    Ok(report)
}

async fn lookup_remote(
    client: &LocalApiClient,
    kind: zsb_core::LibraryKind,
    id: &str,
) -> Result<zsb_core::RemoteLibrary> {
    use zsb_zotero_api::ZoteroSource;
    let libs = client.list_libraries().await?;
    libs.into_iter()
        .find(|l| l.kind == kind && l.zotero_library_id == id)
        .ok_or_else(|| Error::LibraryNotFound(format!("{kind}:{id}")))
}

fn enabled_platforms(config: &Config) -> Vec<Platform> {
    zsb_sync::engine::enabled_platforms(config)
}

// ----------------------------------------------------------------------
// status / doctor / clean-mirrors
// ----------------------------------------------------------------------

fn print_status(db: &Database) -> Result<()> {
    let stats = db.stats()?;
    println!("Zotero Search Bridge 状态");
    println!(
        "  数据库：{}",
        db.path()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    println!(
        "  当前实例：{}",
        db.active_instance_id()?.unwrap_or_else(|| "(未知)".into())
    );
    println!("  文献条目：{}", stats.item_count);
    println!("  文献库数：{}", stats.library_count);
    println!(
        "  最近同步：{}",
        stats.last_sync_at.unwrap_or_else(|| "(从未)".into())
    );
    println!("  待执行镜像任务：{}", stats.pending_jobs);
    if stats.failed_jobs > 0 {
        println!("  失败镜像任务：{}（需检查文件占用）", stats.failed_jobs);
    }
    for lib in db.list_libraries(None)? {
        println!(
            "  - [{}] {} (id={}, 版本 {}, {})",
            lib.remote.kind,
            lib.remote.display_name,
            lib.remote.zotero_library_id,
            lib.state.last_version,
            if lib.state.enabled {
                "启用"
            } else {
                "停用"
            }
        );
        if let Some(err) = &lib.state.last_error {
            println!("      上次错误：{err}");
        }
    }
    Ok(())
}

async fn doctor(config: &Config, db_path: &std::path::Path) {
    let check = |ok: bool, label: String| {
        println!("[{}] {}", if ok { "OK" } else { "FAIL" }, label);
    };

    // 1-4: Zotero probe.
    match LocalApiClient::new(
        &config.zotero.api_base,
        config.zotero.request_timeout_seconds,
    ) {
        Ok(client) => {
            use zsb_zotero_api::ZoteroSource;
            match client.probe().await {
                Ok(info) => {
                    check(true, "Zotero process reachable".into());
                    check(true, "Local API enabled".into());
                    check(
                        true,
                        format!("API version: {}", info.api_version.unwrap_or(0)),
                    );
                    check(
                        !info.server_id.is_empty(),
                        if info.server_id.is_empty() {
                            "Server ID missing (legacy Zotero, using local fallback)".into()
                        } else {
                            "Server ID detected".into()
                        },
                    );
                }
                Err(Error::ApiDisabled) => {
                    check(true, "Zotero process reachable".into());
                    check(
                        false,
                        "Local API disabled: Zotero 设置 → 高级 → 允许其他应用程序通信".into(),
                    );
                }
                Err(e) => {
                    check(false, format!("Zotero process reachable ({e})"));
                }
            }
        }
        Err(e) => check(false, format!("HTTP client init ({e})")),
    }

    // 5: FTS5 availability.
    let fts_ok = Database::open_in_memory()
        .and_then(|db| db.rebuild_fts().map(|_| true))
        .unwrap_or(false);
    check(fts_ok, "SQLite FTS5 available".into());

    // 6: mirror directories writable.
    for platform in enabled_platforms(config) {
        let dir = config.mirror_dir(platform);
        let ok = std::fs::create_dir_all(&dir)
            .and_then(|_| {
                let probe = dir.join(".zsb-write-test");
                std::fs::write(&probe, b"ok")?;
                std::fs::remove_file(&probe)
            })
            .is_ok();
        check(ok, format!("Mirror directory writable: {}", dir.display()));
    }

    // 7: database file usable.
    match Database::open(db_path) {
        Ok(db) => check(
            true,
            format!(
                "Index database ok ({} items)",
                db.stats().map(|s| s.item_count).unwrap_or(0)
            ),
        ),
        Err(e) => check(false, format!("Index database ({e})")),
    }

    // 8: zotero:// handler (Windows registry, best effort).
    check_uri_handler();
}

#[cfg(windows)]
fn check_uri_handler() {
    let ok = std::process::Command::new("reg")
        .args(["query", "HKCR\\zotero", "/ve"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    println!(
        "[{}] zotero:// URI handler registered{}",
        if ok { "OK" } else { "WARN" },
        if ok {
            ""
        } else {
            "（安装并启动一次 Zotero 后自动注册）"
        }
    );
}

#[cfg(not(windows))]
fn check_uri_handler() {
    println!("[INFO] zotero:// handler: macOS 由 LaunchServices 管理，双击任一链接即可验证");
}

fn clean_mirrors(db: &Database, config: &Config) -> Result<()> {
    let mut removed = 0usize;
    let mut kept = 0usize;
    for platform in enabled_platforms(config) {
        let backend = zsb_mirror::backend_for(platform);
        let dir = config.mirror_dir(platform);
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let ext = format!(".{}", backend.extension());
            if !name.ends_with(&ext) {
                continue;
            }
            // Mirror filenames always end with " -- <item_key>.<ext>".
            let Some(stem) = name.strip_suffix(&ext) else {
                continue;
            };
            let Some(pos) = stem.rfind(" -- ") else {
                continue;
            };
            let key = &stem[pos + 4..];
            if key.is_empty() {
                continue;
            }
            if db.item_key_exists(key)? {
                kept += 1;
            } else {
                std::fs::remove_file(entry.path())?;
                removed += 1;
                println!("已删除残留文件：{name}");
            }
        }
    }
    println!("清理完成：删除 {removed} 个残留文件，保留 {kept} 个。");
    Ok(())
}

fn parse_library(input: &str) -> Result<(&str, String)> {
    if input == "user" {
        return Ok(("user", "0".into()));
    }
    if let Some(id) = input.strip_prefix("group:") {
        return Ok(("group", id.to_string()));
    }
    Err(Error::Config(format!(
        "无效的库参数：{input}（应为 user 或 group:<id>）"
    )))
}
