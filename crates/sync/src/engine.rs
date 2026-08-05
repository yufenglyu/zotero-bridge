//! Incremental sync engine (spec section 12).

use crate::normalizer;
use crate::state::{LibrarySyncReport, SyncReport};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, warn};
use zsb_core::{
    Config, Error, MirrorOperation, NewMirrorJob, Platform, RemoteLibrary, Result, SyncBatch,
};
use zsb_index::Database;
use zsb_mirror::{backend_for, filename};
use zsb_zotero_api::discovery::probe_instance;
use zsb_zotero_api::{ZoteroItem, ZoteroSource, BATCH_SIZE};

/// Retries when the library version changes mid-sync (spec section 12.4).
const UNSTABLE_MAX_RETRIES: u32 = 3;
const UNSTABLE_BACKOFF_MS: u64 = 500;

/// Bumped whenever normalization or the preserved `raw_json` shape changes
/// in a way that makes previously indexed rows incomplete (for example when
/// new Zotero fields become available to the filename templater). A mismatch
/// against the persisted value forces one full sync.
/// 1 → initial; 2 → ItemData flatten map started preserving every field.
const NORMALIZER_VERSION: &str = "2";

pub struct SyncEngine<'a, S: ZoteroSource> {
    source: &'a S,
    db: &'a mut Database,
    config: &'a Config,
}

impl<'a, S: ZoteroSource> SyncEngine<'a, S> {
    pub fn new(source: &'a S, db: &'a mut Database, config: &'a Config) -> Self {
        SyncEngine { source, db, config }
    }

    /// Sync every discovered library of the active instance.
    pub async fn sync_all(&mut self, full: bool) -> Result<SyncReport> {
        // When the normalizer/raw_json semantics change (e.g. new preserved
        // fields for the filename templater), rows written by older builds
        // are stale and version-based incremental sync will never refetch
        // them. Force one full sync so every row is rewritten.
        let stored_nv = self.db.meta_get("normalizer_version")?;
        let full = if stored_nv.as_deref() != Some(NORMALIZER_VERSION) {
            info!(
                old = ?stored_nv,
                new = NORMALIZER_VERSION,
                "normalizer version changed; forcing full sync"
            );
            true
        } else {
            full
        };
        // Probe and resolve the server id, with a persisted legacy fallback.
        let legacy = self.db.meta_get("legacy_server_id")?;
        let info = probe_instance(self.source, || {
            // Persisted below on first use.
            legacy.clone().unwrap_or_else(uuid_v4)
        })
        .await?;
        if info.server_id.starts_with("legacy:") && legacy.is_none() {
            self.db.meta_set("legacy_server_id", &info.server_id[7..])?;
        }

        let previous_active = self.db.active_instance_id()?;
        self.db.upsert_instance(&info)?;
        if previous_active.as_deref() != Some(info.server_id.as_str()) {
            info!(
                old = ?previous_active,
                new = %short_id(&info.server_id),
                "zotero instance changed; creating isolated index partition"
            );
            if let Some(old) = &previous_active {
                self.enqueue_instance_mirror_cleanup(old)?;
            }
            self.db.set_active_instance(&info.server_id)?;
        }

        // Discover libraries and filter by configuration.
        let mut remotes = self.source.list_libraries().await?;
        remotes.retain(|lib| match lib.kind {
            zsb_core::LibraryKind::User => self.config.zotero.include_user_library,
            zsb_core::LibraryKind::Group => self.config.zotero.group_mode != "none",
        });

        let mut report = SyncReport {
            server_id: info.server_id.clone(),
            ..Default::default()
        };
        for remote in remotes {
            match self.sync_library(&info.server_id, &remote, full).await {
                Ok(Some(lib_report)) => report.libraries.push(lib_report),
                Ok(None) => debug!(library = %remote.display_name, "library disabled, skipped"),
                Err(e) => {
                    warn!(library = %remote.display_name, error = %e, "library sync failed");
                    if let Ok(id) = self.db.upsert_library(&info.server_id, &remote) {
                        let _ = self.db.record_sync_error(id, &e.to_string());
                    }
                    return Err(e);
                }
            }
        }
        self.db.meta_set("normalizer_version", NORMALIZER_VERSION)?;
        Ok(report)
    }

    /// Sync a single library. Returns `None` when the library is disabled.
    pub async fn sync_library(
        &mut self,
        server_id: &str,
        remote: &RemoteLibrary,
        full: bool,
    ) -> Result<Option<LibrarySyncReport>> {
        let lib_id = self.db.upsert_library(server_id, remote)?;
        let state = self.db.library_state(lib_id)?;
        if !state.enabled {
            return Ok(None);
        }
        let since = if full { 0 } else { state.last_version };

        let mut attempt = 0u32;
        loop {
            match self.fetch_library_changes(remote, since).await? {
                FetchOutcome::Unstable => {
                    attempt += 1;
                    if attempt > UNSTABLE_MAX_RETRIES {
                        return Err(Error::Api {
                            status: 0,
                            message: format!(
                                "library {} kept changing during sync; giving up for this round",
                                remote.api_prefix
                            ),
                        });
                    }
                    debug!(attempt, "library version changed mid-sync; retrying");
                    tokio::time::sleep(Duration::from_millis(UNSTABLE_BACKOFF_MS)).await;
                }
                FetchOutcome::Stable {
                    items,
                    deleted_keys,
                    new_version,
                } => {
                    let report = self
                        .commit_library_changes(
                            lib_id,
                            remote,
                            items,
                            deleted_keys,
                            since,
                            new_version,
                            full,
                        )
                        .await?;
                    return Ok(Some(report));
                }
                FetchOutcome::FullScan { items, remote_keys } => {
                    // No object versions on this server: items missing from
                    // the remote listing are treated as deleted, and the
                    // stored version is not advanced.
                    let local_keys: std::collections::HashSet<String> =
                        self.db.item_keys_for_library(lib_id)?.into_iter().collect();
                    let remote_set: std::collections::HashSet<&String> =
                        remote_keys.iter().collect();
                    let deleted_keys: Vec<String> = local_keys
                        .into_iter()
                        .filter(|k| !remote_set.contains(k))
                        .collect();
                    let report = self
                        .commit_library_changes(
                            lib_id,
                            remote,
                            items,
                            deleted_keys,
                            since,
                            since,
                            full,
                        )
                        .await?;
                    return Ok(Some(report));
                }
            }
        }
    }

    /// Fetch changed items and deletions; verify that all responses agree
    /// on `Last-Modified-Version` (spec section 12.4).
    ///
    /// Fallback: some Zotero builds (e.g. 10.0 betas) do not expose object
    /// versions (`format=versions` returns empty values and
    /// `Last-Modified-Version: 0`). For those we do a full item listing and
    /// let content hashes detect changes, computing deletions by comparing
    /// the remote key set with the local one.
    async fn fetch_library_changes(
        &self,
        remote: &RemoteLibrary,
        since: u64,
    ) -> Result<FetchOutcome> {
        let mut versions = self.source.changed_item_versions(remote, since).await?;
        let versions_unavailable =
            versions.last_modified_version == 0 && versions.versions.values().all(|v| *v == 0);
        if versions_unavailable && since > 0 {
            // Re-request the full key listing; `since` filtering is not
            // meaningful without versions.
            versions = self.source.changed_item_versions(remote, 0).await?;
        }
        if versions_unavailable {
            return self.fetch_full_scan(remote, versions.versions).await;
        }

        let keys: Vec<String> = versions.versions.keys().cloned().collect();

        let mut items: Vec<ZoteroItem> = Vec::with_capacity(keys.len());
        let mut observed_versions: Vec<u64> = vec![versions.last_modified_version];
        for chunk in keys.chunks(BATCH_SIZE) {
            let resp = self.source.fetch_items(remote, chunk).await?;
            observed_versions.push(resp.last_modified_version);
            items.extend(resp.items);
        }

        let deleted = self.source.deleted_objects(remote, since).await?;
        observed_versions.push(deleted.last_modified_version);

        // Ignore zero (missing header, e.g. mocks) when comparing.
        let mut distinct: Vec<u64> = observed_versions.into_iter().filter(|v| *v > 0).collect();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() > 1 {
            return Ok(FetchOutcome::Unstable);
        }

        let mut new_version = distinct.first().copied().unwrap_or(0);
        new_version = new_version.max(versions.versions.values().copied().max().unwrap_or(0));
        new_version = new_version.max(since);

        Ok(FetchOutcome::Stable {
            items,
            deleted_keys: deleted.deleted.items,
            new_version,
        })
    }

    /// Full-scan fallback for servers without object versions: fetch every
    /// top-level item and diff by content hash; items missing remotely are
    /// treated as deleted. `new_version` stays at the previous value so the
    /// next round performs the same full scan.
    async fn fetch_full_scan(
        &self,
        remote: &RemoteLibrary,
        versions: zsb_core::VersionMap,
    ) -> Result<FetchOutcome> {
        let keys: Vec<String> = versions.keys().cloned().collect();
        let mut items: Vec<ZoteroItem> = Vec::with_capacity(keys.len());
        for chunk in keys.chunks(BATCH_SIZE) {
            let resp = self.source.fetch_items(remote, chunk).await?;
            items.extend(resp.items);
        }
        Ok(FetchOutcome::FullScan {
            items,
            remote_keys: keys,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_library_changes(
        &mut self,
        lib_id: i64,
        remote: &RemoteLibrary,
        items: Vec<ZoteroItem>,
        mut deleted_keys: Vec<String>,
        since: u64,
        new_version: u64,
        full: bool,
    ) -> Result<LibrarySyncReport> {
        // Gather existing mirror filenames for every touched key so we can
        // plan create / rename / delete jobs correctly.
        let all_keys: Vec<String> = items
            .iter()
            .map(|i| i.key.clone())
            .chain(deleted_keys.iter().cloned())
            .collect();
        let existing_names = self.db.mirror_filenames(lib_id, &all_keys)?;

        let mut upserts = Vec::new();
        let mut skipped = 0usize;
        for raw in &items {
            match normalizer::normalize_item(
                raw,
                remote,
                lib_id,
                self.config.search.store_raw_json,
                self.config.search.index_abstract,
                self.config.search.index_extra,
            ) {
                Some(item) => {
                    // Skip no-op updates identified by content hash. The
                    // preserved raw_json is compared as well: it is not part
                    // of the content hash, and rows written before new fields
                    // were preserved (see NORMALIZER_VERSION) must be
                    // rewritten on the forced full sync.
                    if let Some(old) = self.db.get_item(lib_id, &item.item_key)? {
                        if old.content_hash == item.content_hash
                            && old.item_version == item.item_version
                            && old.raw_json == item.raw_json
                        {
                            skipped += 1;
                            continue;
                        }
                    }
                    upserts.push(item);
                }
                None => {
                    // Excluded type or trashed: remove from index if present.
                    if !deleted_keys.contains(&raw.key) {
                        deleted_keys.push(raw.key.clone());
                    }
                }
            }
        }

        let mirror_jobs =
            plan_mirror_jobs(self.config, &mut upserts, &deleted_keys, &existing_names);

        let batch = SyncBatch {
            new_version,
            deleted_keys: deleted_keys.clone(),
            mirror_jobs,
            upserts,
        };
        let job_count = batch.mirror_jobs.len();
        let upsert_count = batch.upserts.len();
        let delete_count = batch.deleted_keys.len();
        self.db
            .apply_sync_batch(lib_id, &batch, self.config.search.store_raw_json)?;

        info!(
            library = %remote.display_name,
            upserts = upsert_count,
            deletes = delete_count,
            version = new_version,
            "library synced"
        );

        Ok(LibrarySyncReport {
            library_kind: remote.kind,
            zotero_library_id: remote.zotero_library_id.clone(),
            display_name: remote.display_name.clone(),
            upserted: upsert_count,
            deleted: delete_count,
            mirror_jobs: job_count,
            skipped_unchanged: skipped,
            from_version: since,
            to_version: new_version,
            full,
        })
    }

    /// When the Zotero instance changes, remove mirror files that belong to
    /// the old instance (spec section 19.3).
    fn enqueue_instance_mirror_cleanup(&mut self, old_server_id: &str) -> Result<()> {
        let names = self.db.all_mirror_filenames_for_server(old_server_id)?;
        if names.is_empty() {
            return Ok(());
        }
        let mut jobs = Vec::new();
        for platform in enabled_platforms(self.config) {
            let backend = backend_for(platform);
            let dir = self.config.mirror_dir(platform);
            for name in &names {
                jobs.push(NewMirrorJob {
                    operation: MirrorOperation::Delete,
                    platform,
                    old_path: Some(
                        dir.join(format!("{name}.{}", backend.extension()))
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    new_path: None,
                    content: None,
                });
            }
        }
        self.db.enqueue_jobs(&jobs)?;
        Ok(())
    }
}

enum FetchOutcome {
    Unstable,
    Stable {
        items: Vec<ZoteroItem>,
        deleted_keys: Vec<String>,
        new_version: u64,
    },
    /// Full-scan result for servers without object versions.
    FullScan {
        items: Vec<ZoteroItem>,
        remote_keys: Vec<String>,
    },
}

/// Platforms with mirroring enabled in the configuration.
pub fn enabled_platforms(config: &Config) -> Vec<Platform> {
    let mut out = Vec::new();
    if config.mirror.windows.enabled {
        out.push(Platform::Windows);
    }
    if config.mirror.macos.enabled {
        out.push(Platform::Macos);
    }
    out
}

/// Plan mirror jobs for one sync batch (spec section 13). Also stamps
/// each upserted item with its rendered mirror base filename.
fn plan_mirror_jobs(
    config: &Config,
    upserts: &mut [zsb_core::IndexedItem],
    deleted_keys: &[String],
    existing_names: &HashMap<String, String>,
) -> Vec<NewMirrorJob> {
    let mut jobs = Vec::new();
    let platforms = enabled_platforms(config);

    // Render the canonical base filename once per item (first enabled
    // platform's template) and store it on the item. The effective
    // template follows Zotero's own attachment rename template unless the
    // user configured a custom one (zsb_core::zotero_prefs).
    if let Some(first) = platforms.first() {
        let template =
            zsb_core::zotero_prefs::resolve_template(&config.mirror_for(*first).template);
        for item in upserts.iter_mut() {
            item.mirror_filename = Some(filename::render_auto(&template, item));
        }

        // Collision fallback: Zotero-style templates carry no item key, so
        // two items can render to the same base name and fight over one
        // file. Append the item key to later duplicates (existing names of
        // untouched items count as taken too).
        let mut taken: HashMap<String, String> = HashMap::new();
        for (k, v) in existing_names {
            taken.entry(v.clone()).or_insert_with(|| k.clone());
        }
        for item in upserts.iter_mut() {
            let Some(base) = item.mirror_filename.clone() else {
                continue;
            };
            match taken.get(&base) {
                Some(owner) if *owner != item.item_key => {
                    let unique = filename::with_key_suffix(&base, &item.item_key);
                    taken.insert(unique.clone(), item.item_key.clone());
                    item.mirror_filename = Some(unique);
                }
                _ => {
                    taken.insert(base, item.item_key.clone());
                }
            }
        }
    }

    for platform in platforms {
        let backend = backend_for(platform);
        let template =
            zsb_core::zotero_prefs::resolve_template(&config.mirror_for(platform).template);
        let dir = config.mirror_dir(platform);

        for item in upserts.iter() {
            let base = item
                .mirror_filename
                .clone()
                .unwrap_or_else(|| filename::render_auto(&template, item));
            let new_path = dir
                .join(format!("{base}.{}", backend.extension()))
                .to_string_lossy()
                .into_owned();
            let content = backend.build_content(&item.select_uri);
            match existing_names.get(&item.item_key) {
                Some(old_base) if *old_base != base => jobs.push(NewMirrorJob {
                    operation: MirrorOperation::Rename,
                    platform,
                    old_path: Some(
                        dir.join(format!("{old_base}.{}", backend.extension()))
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    new_path: Some(new_path),
                    content: Some(content),
                }),
                Some(_) => {} // Filename (and URI) unchanged: nothing to do.
                None => jobs.push(NewMirrorJob {
                    operation: MirrorOperation::Create,
                    platform,
                    old_path: None,
                    new_path: Some(new_path),
                    content: Some(content),
                }),
            }
        }

        for key in deleted_keys {
            if let Some(old_base) = existing_names.get(key) {
                jobs.push(NewMirrorJob {
                    operation: MirrorOperation::Delete,
                    platform,
                    old_path: Some(
                        dir.join(format!("{old_base}.{}", backend.extension()))
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    new_path: None,
                    content: None,
                });
            }
        }
    }
    jobs
}

fn short_id(server_id: &str) -> String {
    server_id.chars().take(8).collect()
}

/// Outcome of a mirror refresh pass.
#[derive(Debug, Clone, Default)]
pub struct MirrorRefreshReport {
    pub renamed: usize,
    pub rewritten: usize,
    pub unchanged: usize,
}

/// Re-render every indexed item's mirror filename with the current
/// template and enqueue the jobs needed to bring the shortcut directory
/// back in sync with the database: Rename for changed names, Create for
/// files missing on disk. Stored filenames and jobs commit in the same
/// transaction (outbox pattern), so a crash mid-refresh is recoverable.
pub fn refresh_mirrors(db: &mut Database, config: &Config) -> Result<MirrorRefreshReport> {
    let platforms = enabled_platforms(config);
    if platforms.is_empty() {
        return Ok(MirrorRefreshReport::default());
    }
    let template =
        zsb_core::zotero_prefs::resolve_template(&config.mirror_for(platforms[0]).template);
    let mut report = MirrorRefreshReport::default();

    for lib in db.list_libraries(None)? {
        let items = db.items_for_library(lib.id)?;
        if items.is_empty() {
            continue;
        }

        // Re-render every item, deduplicating against all names in the
        // library (same collision fallback as the sync planner).
        let mut taken: HashMap<String, String> = HashMap::new();
        let mut new_names: Vec<String> = Vec::with_capacity(items.len());
        for item in &items {
            let mut base = filename::render_auto(&template, item);
            if taken
                .get(&base)
                .is_some_and(|owner| *owner != item.item_key)
            {
                base = filename::with_key_suffix(&base, &item.item_key);
            }
            taken.insert(base.clone(), item.item_key.clone());
            new_names.push(base);
        }

        let mut upserts = Vec::new();
        let mut jobs = Vec::new();
        for (item, base) in items.iter().zip(new_names) {
            let old = item.mirror_filename.clone();
            let name_changed = old.as_deref() != Some(base.as_str());
            for &platform in &platforms {
                let backend = backend_for(platform);
                let dir = config.mirror_dir(platform);
                let new_path = dir.join(format!("{base}.{}", backend.extension()));
                if !name_changed && new_path.exists() {
                    continue;
                }
                let content = backend.build_content(&item.select_uri);
                let new_path_s = new_path.to_string_lossy().into_owned();
                if name_changed && old.is_some() {
                    let old_path = dir
                        .join(format!(
                            "{}.{}",
                            old.as_deref().unwrap_or_default(),
                            backend.extension()
                        ))
                        .to_string_lossy()
                        .into_owned();
                    jobs.push(NewMirrorJob {
                        operation: MirrorOperation::Rename,
                        platform,
                        old_path: Some(old_path),
                        new_path: Some(new_path_s),
                        content: Some(content),
                    });
                    report.renamed += 1;
                } else {
                    jobs.push(NewMirrorJob {
                        operation: MirrorOperation::Create,
                        platform,
                        old_path: None,
                        new_path: Some(new_path_s),
                        content: Some(content),
                    });
                    report.rewritten += 1;
                }
            }
            if name_changed {
                let mut updated = item.clone();
                updated.mirror_filename = Some(base);
                upserts.push(updated);
            } else {
                report.unchanged += 1;
            }
        }

        if upserts.is_empty() && jobs.is_empty() {
            continue;
        }
        let version = db.library_state(lib.id)?.last_version;
        db.apply_sync_batch(
            lib.id,
            &SyncBatch {
                new_version: version,
                deleted_keys: vec![],
                mirror_jobs: jobs,
                upserts,
            },
            config.search.store_raw_json,
        )?;
    }
    Ok(report)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zsb_core::LibraryKind;

    /// Custom template for tests: renders identically to the built-in
    /// default but differs as a string, so template resolution treats it
    /// as custom and never reads the developer machine's real Zotero pref
    /// (which would make these tests environment-dependent).
    const TEST_TEMPLATE: &str =
        "{primary_creator} - {year} - {title}{container_title} -- {item_key}";

    fn test_config() -> Config {
        let mut cfg = Config::default();
        cfg.mirror.windows.enabled = true;
        cfg.mirror.windows.directory = "/tmp/zsb-test-mirrors".into();
        cfg.mirror.windows.template = TEST_TEMPLATE.into();
        cfg.mirror.macos.enabled = false;
        cfg
    }

    #[test]
    fn plans_create_rename_delete_jobs() {
        let cfg = test_config();
        let item = |key: &str, creator: &str| zsb_core::IndexedItem {
            item_key: key.into(),
            primary_creator: creator.into(),
            year: "2024".into(),
            title: "燃气轮机研究".into(),
            select_uri: format!("zotero://select/library/items/{key}"),
            ..Default::default()
        };
        let mut upserts = vec![item("NEW00001", "张三"), item("CHG00002", "李四")];
        let mut existing = HashMap::new();
        existing.insert(
            "CHG00002".to_string(),
            "王五 - 2024 - 燃气轮机研究 -- CHG00002".to_string(),
        );
        existing.insert(
            "DEL00003".to_string(),
            "赵六 - 2023 - 旧文 -- DEL00003".to_string(),
        );
        let jobs = plan_mirror_jobs(&cfg, &mut upserts, &["DEL00003".into()], &existing);
        assert_eq!(jobs.len(), 3);
        assert!(jobs.iter().any(|j| j.operation == MirrorOperation::Create
            && j.new_path.as_ref().unwrap().contains("NEW00001.url")));
        assert!(jobs.iter().any(|j| j.operation == MirrorOperation::Rename
            && j.old_path.as_ref().unwrap().contains("王五")
            && j.new_path.as_ref().unwrap().contains("李四")));
        assert!(jobs.iter().any(|j| j.operation == MirrorOperation::Delete
            && j.old_path.as_ref().unwrap().contains("DEL00003")));
    }

    #[test]
    fn no_job_when_filename_unchanged() {
        let cfg = test_config();
        let item = zsb_core::IndexedItem {
            item_key: "SAME0001".into(),
            primary_creator: "张三".into(),
            year: "2024".into(),
            title: "燃气轮机研究".into(),
            select_uri: "zotero://select/library/items/SAME0001".into(),
            ..Default::default()
        };
        let mut existing = HashMap::new();
        existing.insert(
            "SAME0001".to_string(),
            filename::render(&cfg.mirror.windows.template, &item),
        );
        let jobs = plan_mirror_jobs(&cfg, &mut [item], &[], &existing);
        assert!(jobs.is_empty());
    }

    #[test]
    fn disabled_mirrors_plan_nothing() {
        let mut cfg = Config::default();
        cfg.mirror.windows.enabled = false;
        cfg.mirror.macos.enabled = false;
        let jobs = plan_mirror_jobs(&cfg, &mut [], &[], &HashMap::new());
        assert!(jobs.is_empty());
        let _ = LibraryKind::User;
    }

    #[test]
    fn zotero_template_collisions_get_key_suffix() {
        let mut cfg = test_config();
        cfg.mirror.windows.template = "【{{authors}}】{{title}}".into();
        let item = |key: &str| zsb_core::IndexedItem {
            item_key: key.into(),
            primary_creator: "张三".into(),
            year: "2024".into(),
            title: "燃气轮机研究".into(),
            select_uri: format!("zotero://select/library/items/{key}"),
            ..Default::default()
        };
        let mut upserts = vec![item("AAAA0001"), item("BBBB0002")];
        let jobs = plan_mirror_jobs(&cfg, &mut upserts, &[], &HashMap::new());
        // Identical render output: the second item must be disambiguated.
        assert_eq!(
            upserts[0].mirror_filename.as_deref(),
            Some("【张三】燃气轮机研究")
        );
        assert_eq!(
            upserts[1].mirror_filename.as_deref(),
            Some("【张三】燃气轮机研究 -- BBBB0002")
        );
        assert_eq!(jobs.len(), 2);
    }

    #[test]
    fn refresh_renames_changed_and_rewrites_missing() {
        let dir = std::env::temp_dir().join(format!("zsb-refresh-{}", uuid_v4()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = test_config();
        cfg.mirror.windows.directory = dir.to_string_lossy().into_owned();

        let mut db = Database::open_in_memory().unwrap();
        db.upsert_instance(&zsb_core::ServerInfo {
            api_base: "http://localhost:23119/api".into(),
            api_version: Some(3),
            schema_version: None,
            server_id: "srv".into(),
        })
        .unwrap();
        let lib_id = db.upsert_library("srv", &RemoteLibrary::user()).unwrap();

        let item = |key: &str, creator: &str, stored: &str| zsb_core::IndexedItem {
            item_key: key.into(),
            item_version: 1,
            item_type: "journalArticle".into(),
            title: "燃气轮机研究".into(),
            primary_creator: creator.into(),
            year: "2024".into(),
            select_uri: format!("zotero://select/library/items/{key}"),
            mirror_filename: Some(stored.into()),
            content_hash: format!("h-{key}"),
            ..Default::default()
        };
        // A: stored name is stale -> rename. B: stored name matches the
        // current template but the file is missing -> rewrite.
        let a = item("AAAA0001", "张三", "旧名字 -- AAAA0001");
        let b_rendered = filename::render_auto(
            &cfg.mirror.windows.template,
            &item("BBBB0002", "李四", "ignored"),
        );
        let b = item("BBBB0002", "李四", &b_rendered);
        db.apply_sync_batch(
            lib_id,
            &SyncBatch {
                new_version: 1,
                deleted_keys: vec![],
                mirror_jobs: vec![],
                upserts: vec![a, b],
            },
            true,
        )
        .unwrap();
        // Only A's file exists on disk (stale name).
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("旧名字 -- AAAA0001.url"), "x").unwrap();

        let report = refresh_mirrors(&mut db, &cfg).unwrap();
        assert_eq!(report.renamed, 1);
        assert_eq!(report.rewritten, 1);

        zsb_mirror::worker::process_pending(&db, Platform::Windows, 100).unwrap();
        assert!(!dir.join("旧名字 -- AAAA0001.url").exists());
        assert!(dir
            .join("张三 - 2024 - 燃气轮机研究 -- AAAA0001.url")
            .exists());
        assert!(dir
            .join("李四 - 2024 - 燃气轮机研究 -- BBBB0002.url")
            .exists());

        // Second refresh is a no-op.
        let report = refresh_mirrors(&mut db, &cfg).unwrap();
        assert_eq!((report.renamed, report.rewritten), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
