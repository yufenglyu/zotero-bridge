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

pub struct SyncEngine<'a, S: ZoteroSource> {
    source: &'a S,
    db: &'a mut Database,
    config: &'a Config,
}

impl<'a, S: ZoteroSource> SyncEngine<'a, S> {
    pub fn new(source: &'a S, db: &'a mut Database, config: &'a Config) -> Self {
        SyncEngine {
            source,
            db,
            config,
        }
    }

    /// Sync every discovered library of the active instance.
    pub async fn sync_all(&mut self, full: bool) -> Result<SyncReport> {
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
                            lib_id, remote, items, deleted_keys, since, new_version, full,
                        )
                        .await?;
                    return Ok(Some(report));
                }
                FetchOutcome::FullScan { items, remote_keys } => {
                    // No object versions on this server: items missing from
                    // the remote listing are treated as deleted, and the
                    // stored version is not advanced.
                    let local_keys: std::collections::HashSet<String> = self
                        .db
                        .item_keys_for_library(lib_id)?
                        .into_iter()
                        .collect();
                    let remote_set: std::collections::HashSet<&String> =
                        remote_keys.iter().collect();
                    let deleted_keys: Vec<String> = local_keys
                        .into_iter()
                        .filter(|k| !remote_set.contains(k))
                        .collect();
                    let report = self
                        .commit_library_changes(
                            lib_id, remote, items, deleted_keys, since, since, full,
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
        let versions_unavailable = versions.last_modified_version == 0
            && versions.versions.values().all(|v| *v == 0);
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
        let mut distinct: Vec<u64> = observed_versions
            .into_iter()
            .filter(|v| *v > 0)
            .collect();
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
                    // Skip no-op updates identified by content hash.
                    if let Some(old) = self.db.get_item(lib_id, &item.item_key)? {
                        if old.content_hash == item.content_hash
                            && old.item_version == item.item_version
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

        let mirror_jobs = plan_mirror_jobs(
            self.config,
            &mut upserts,
            &deleted_keys,
            &existing_names,
        );

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
    // platform's template) and store it on the item.
    if let Some(first) = platforms.first() {
        let template = &config.mirror_for(*first).template;
        for item in upserts.iter_mut() {
            item.mirror_filename = Some(filename::render(template, item));
        }
    }

    for platform in platforms {
        let backend = backend_for(platform);
        let template = &config.mirror_for(platform).template;
        let dir = config.mirror_dir(platform);

        for item in upserts.iter() {
            let base = item
                .mirror_filename
                .clone()
                .unwrap_or_else(|| filename::render(template, item));
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

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zsb_core::LibraryKind;

    fn test_config() -> Config {
        let mut cfg = Config::default();
        cfg.mirror.windows.enabled = true;
        cfg.mirror.windows.directory = "/tmp/zsb-test-mirrors".into();
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
        assert!(jobs
            .iter()
            .any(|j| j.operation == MirrorOperation::Create
                && j.new_path.as_ref().unwrap().contains("NEW00001.url")));
        assert!(jobs
            .iter()
            .any(|j| j.operation == MirrorOperation::Rename
                && j.old_path.as_ref().unwrap().contains("王五")
                && j.new_path.as_ref().unwrap().contains("李四")));
        assert!(jobs
            .iter()
            .any(|j| j.operation == MirrorOperation::Delete
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
}
