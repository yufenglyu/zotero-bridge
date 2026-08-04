//! Integration tests: sync engine against a mock Zotero data source
//! (spec section 21.2).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use zsb_core::{Config, LibraryKind, Platform, RemoteLibrary, Result, ServerInfo, VersionMap};
use zsb_index::Database;
use zsb_sync::SyncEngine;
use zsb_zotero_api::source::{DeletedObjects, ItemResponse, VersionResponse};
use zsb_zotero_api::{DeletedResponse, ZoteroItem, ZoteroSource};

/// In-memory Zotero stand-in. The user library and each group library
/// hold separate item sets, like the real API.
struct MockSource {
    server_id: String,
    library_version: u64,
    items: HashMap<String, ZoteroItem>,
    deleted: Vec<String>,
    groups: Vec<RemoteLibrary>,
    group_items: HashMap<String, HashMap<String, ZoteroItem>>,
    /// When >0, the next N sync attempts report inconsistent
    /// Last-Modified-Version headers to simulate concurrent edits.
    unstable_rounds: Mutex<u32>,
}

impl MockSource {
    fn new(items: Vec<ZoteroItem>) -> Self {
        let mut map = HashMap::new();
        let mut version = 0u64;
        for item in items {
            version = version.max(item.version);
            map.insert(item.key.clone(), item);
        }
        MockSource {
            server_id: "mock-server-1".into(),
            library_version: version,
            items: map,
            deleted: Vec::new(),
            groups: Vec::new(),
            group_items: HashMap::new(),
            unstable_rounds: Mutex::new(0),
        }
    }

    fn add_group(&mut self, group: RemoteLibrary, items: Vec<ZoteroItem>) {
        let mut map = HashMap::new();
        for item in items {
            self.library_version = self.library_version.max(item.version);
            map.insert(item.key.clone(), item);
        }
        self.group_items
            .insert(group.zotero_library_id.clone(), map);
        self.groups.push(group);
    }

    fn items_for<'a>(&'a self, library: &RemoteLibrary) -> &'a HashMap<String, ZoteroItem> {
        match library.kind {
            LibraryKind::User => &self.items,
            LibraryKind::Group => self
                .group_items
                .get(&library.zotero_library_id)
                .unwrap_or(&self.items),
        }
    }
}

fn item_json(key: &str, version: u64, item_type: &str, title: &str) -> ZoteroItem {
    serde_json::from_value(serde_json::json!({
        "key": key,
        "version": version,
        "data": {
            "itemType": item_type,
            "title": title,
            "creators": [{"creatorType": "author", "firstName": "Wei", "lastName": "Wang"}],
            "date": "2024-01-15",
            "publicationTitle": "Journal of Testing",
            "tags": [{"tag": "仿真"}],
            "dateModified": "2024-01-16T00:00:00Z"
        }
    }))
    .unwrap()
}

impl ZoteroSource for MockSource {
    async fn probe(&self) -> Result<ServerInfo> {
        Ok(ServerInfo {
            api_base: "mock://local".into(),
            api_version: Some(3),
            schema_version: Some(50),
            server_id: self.server_id.clone(),
        })
    }

    async fn list_libraries(&self) -> Result<Vec<RemoteLibrary>> {
        let mut libs = vec![RemoteLibrary::user()];
        libs.extend(self.groups.iter().cloned());
        Ok(libs)
    }

    async fn changed_item_versions(
        &self,
        library: &RemoteLibrary,
        since: u64,
    ) -> Result<VersionResponse> {
        let versions: VersionMap = self
            .items_for(library)
            .values()
            .filter(|i| i.version > since)
            .map(|i| (i.key.clone(), i.version))
            .collect();
        Ok(VersionResponse {
            versions,
            last_modified_version: self.library_version,
        })
    }

    async fn fetch_items(&self, library: &RemoteLibrary, keys: &[String]) -> Result<ItemResponse> {
        let items: Vec<ZoteroItem> = keys
            .iter()
            .filter_map(|k| self.items_for(library).get(k).cloned())
            .collect();
        // Simulate an unstable library for the configured number of rounds:
        // fetch_items reports a *different* version than the other calls.
        let mut rounds = self.unstable_rounds.lock().unwrap();
        let lmv = if *rounds > 0 {
            *rounds -= 1;
            self.library_version + 100
        } else {
            self.library_version
        };
        Ok(ItemResponse {
            items,
            last_modified_version: lmv,
        })
    }

    async fn deleted_objects(
        &self,
        _library: &RemoteLibrary,
        _since: u64,
    ) -> Result<DeletedObjects> {
        Ok(DeletedObjects {
            deleted: DeletedResponse {
                items: self.deleted.clone(),
                ..Default::default()
            },
            last_modified_version: self.library_version,
        })
    }
}

fn test_config() -> Config {
    let mut cfg = Config::default();
    cfg.mirror.windows.enabled = true;
    cfg.mirror.windows.directory = std::env::temp_dir()
        .join(format!("zsb-it-{}", std::process::id()))
        .to_string_lossy()
        .into_owned();
    cfg.mirror.macos.enabled = false;
    cfg
}

async fn synced_db(source: &MockSource, cfg: &Config) -> (Database, zsb_sync::SyncReport) {
    let mut db = Database::open_in_memory().unwrap();
    let report = {
        let mut engine = SyncEngine::new(source, &mut db, cfg);
        engine.sync_all(false).await.unwrap()
    };
    (db, report)
}

fn search(db: &Database, q: &str) -> Vec<zsb_core::SearchResult> {
    zsb_index::search::search(db.connection(), &zsb_index::build_search_query(q, 30)).unwrap()
}

#[tokio::test]
async fn initial_sync_indexes_items_and_plans_mirrors() {
    let source = MockSource::new(vec![
        item_json("AAAA1111", 5, "journalArticle", "燃气轮机转子动力学研究"),
        item_json("BBBB2222", 7, "book", "Turbine Blade Cooling"),
        item_json("ATT00003", 7, "attachment", "PDF file"), // excluded
    ]);
    let cfg = test_config();
    let (db, report) = synced_db(&source, &cfg).await;

    assert_eq!(db.count_items().unwrap(), 2);
    assert_eq!(report.libraries.len(), 1);
    assert_eq!(report.libraries[0].to_version, 7);
    assert_eq!(search(&db, "燃气轮机").len(), 1);
    assert_eq!(search(&db, "cooling").len(), 1);

    // Two create jobs (attachment excluded).
    let jobs = db.pending_jobs(Platform::Windows, 100).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs
        .iter()
        .all(|j| j.operation == zsb_core::MirrorOperation::Create));
}

#[tokio::test]
async fn incremental_sync_picks_up_changes() {
    let mut source = MockSource::new(vec![item_json(
        "AAAA1111",
        5,
        "journalArticle",
        "燃气轮机转子动力学研究",
    )]);
    let cfg = test_config();
    let (mut db, _) = synced_db(&source, &cfg).await;
    assert_eq!(search(&db, "燃气轮机").len(), 1);

    // User edits the title in Zotero; version bumps.
    source.items.insert(
        "AAAA1111".into(),
        item_json("AAAA1111", 9, "journalArticle", "航空发动机叶片振动研究"),
    );
    source.library_version = 9;

    let report = {
        let mut engine = SyncEngine::new(&source, &mut db, &cfg);
        engine.sync_all(false).await.unwrap()
    };
    assert_eq!(report.libraries[0].upserted, 1);
    assert!(search(&db, "燃气轮机").is_empty());
    assert_eq!(search(&db, "叶片").len(), 1);

    // A rename job was planned because the title (filename) changed.
    let jobs = db.pending_jobs(Platform::Windows, 100).unwrap();
    assert!(jobs
        .iter()
        .any(|j| j.operation == zsb_core::MirrorOperation::Rename));
}

#[tokio::test]
async fn deleted_items_disappear_from_index() {
    let mut source = MockSource::new(vec![
        item_json("AAAA1111", 5, "journalArticle", "研究甲"),
        item_json("BBBB2222", 5, "journalArticle", "研究乙"),
    ]);
    let cfg = test_config();
    let (mut db, _) = synced_db(&source, &cfg).await;
    assert_eq!(db.count_items().unwrap(), 2);

    source.items.remove("AAAA1111");
    source.deleted.push("AAAA1111".into());
    source.library_version = 6;

    {
        let mut engine = SyncEngine::new(&source, &mut db, &cfg);
        engine.sync_all(false).await.unwrap();
    }
    assert_eq!(db.count_items().unwrap(), 1);
    assert!(search(&db, "研究甲").is_empty());
    let jobs = db.pending_jobs(Platform::Windows, 100).unwrap();
    assert!(jobs
        .iter()
        .any(|j| j.operation == zsb_core::MirrorOperation::Delete));
}

#[tokio::test]
async fn trashed_items_are_removed() {
    let mut source = MockSource::new(vec![item_json(
        "AAAA1111",
        5,
        "journalArticle",
        "被回收的研究",
    )]);
    let cfg = test_config();
    let (mut db, _) = synced_db(&source, &cfg).await;
    assert_eq!(db.count_items().unwrap(), 1);

    // Item moves to trash: still returned by the API (includeTrashed=1)
    // with deleted=1 and a bumped version.
    source.items.insert(
        "AAAA1111".into(),
        serde_json::from_value(serde_json::json!({
            "key": "AAAA1111",
            "version": 6,
            "data": {"itemType": "journalArticle", "title": "被回收的研究", "deleted": 1}
        }))
        .unwrap(),
    );
    source.library_version = 6;

    {
        let mut engine = SyncEngine::new(&source, &mut db, &cfg);
        engine.sync_all(false).await.unwrap();
    }
    assert_eq!(db.count_items().unwrap(), 0);
}

#[tokio::test]
async fn group_library_uses_group_select_uri() {
    let mut source = MockSource::new(vec![]);
    source.add_group(
        RemoteLibrary::group("123456", "团队库"),
        vec![item_json("GRP00001", 3, "journalArticle", "群组库文献")],
    );
    let cfg = test_config();
    let (db, report) = synced_db(&source, &cfg).await;
    assert_eq!(report.libraries.len(), 2);

    let hits = search(&db, "群组库文献");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].select_uri,
        "zotero://select/groups/123456/items/GRP00001"
    );
    assert_eq!(hits[0].library_kind, LibraryKind::Group);
}

#[tokio::test]
async fn unstable_library_version_retries_then_commits() {
    let source = MockSource::new(vec![item_json(
        "AAAA1111",
        5,
        "journalArticle",
        "并发修改测试",
    )]);
    *source.unstable_rounds.lock().unwrap() = 2;
    let cfg = test_config();
    let (_db, report) = synced_db(&source, &cfg).await;
    assert_eq!(report.libraries[0].upserted, 1);
}

#[tokio::test]
async fn new_instance_gets_isolated_partition() {
    let source = MockSource::new(vec![item_json(
        "AAAA1111",
        5,
        "journalArticle",
        "实例一切换前",
    )]);
    let cfg = test_config();
    let (mut db, _) = synced_db(&source, &cfg).await;
    assert_eq!(
        db.active_instance_id().unwrap().as_deref(),
        Some("mock-server-1")
    );

    // Zotero database replaced: new server id, fresh library.
    let mut source2 = MockSource::new(vec![item_json(
        "NEW00001",
        1,
        "journalArticle",
        "实例二全新文献",
    )]);
    source2.server_id = "mock-server-2".into();
    {
        let mut engine = SyncEngine::new(&source2, &mut db, &cfg);
        engine.sync_all(false).await.unwrap();
    }
    assert_eq!(
        db.active_instance_id().unwrap().as_deref(),
        Some("mock-server-2")
    );
    // Old instance data is preserved, new instance has its own rows.
    assert_eq!(db.count_items().unwrap(), 2);
    // Mirror cleanup jobs for the old instance were enqueued.
    let jobs = db.pending_jobs(Platform::Windows, 100).unwrap();
    assert!(jobs
        .iter()
        .any(|j| j.operation == zsb_core::MirrorOperation::Delete));
}

#[tokio::test]
async fn empty_library_syncs_cleanly() {
    let source = MockSource::new(vec![]);
    let cfg = test_config();
    let (db, report) = synced_db(&source, &cfg).await;
    assert_eq!(db.count_items().unwrap(), 0);
    assert_eq!(report.libraries.len(), 1);
}

#[tokio::test]
async fn batch_boundary_50_keys() {
    // 55 items forces two fetch_items batches (spec: max 50 keys each).
    let items: Vec<ZoteroItem> = (0..55)
        .map(|i| {
            item_json(
                &format!("KEY{i:05}"),
                1,
                "journalArticle",
                &format!("批量测试文献 {i}"),
            )
        })
        .collect();
    let source = MockSource::new(items);
    let cfg = test_config();
    let (db, _) = synced_db(&source, &cfg).await;
    assert_eq!(db.count_items().unwrap(), 55);
    assert_eq!(search(&db, "批量测试").len(), 30); // limited to 30
}

/// A second sync with no changes should be a no-op (content hash skip).
#[tokio::test]
async fn resync_without_changes_is_noop() {
    let source = MockSource::new(vec![item_json(
        "AAAA1111",
        5,
        "journalArticle",
        "不变的文献",
    )]);
    let cfg = test_config();
    let (mut db, _) = synced_db(&source, &cfg).await;
    let report = {
        let mut engine = SyncEngine::new(&source, &mut db, &cfg);
        engine.sync_all(false).await.unwrap()
    };
    assert_eq!(report.libraries[0].upserted, 0);
    assert_eq!(report.libraries[0].deleted, 0);
}

/// Legacy Zotero without Server-ID header falls back to a persisted id.
#[tokio::test]
async fn legacy_server_id_fallback_is_stable() {
    let mut source = MockSource::new(vec![]);
    source.server_id = String::new(); // header missing
    let cfg = test_config();
    let mut db = Database::open_in_memory().unwrap();
    {
        let mut engine = SyncEngine::new(&source, &mut db, &cfg);
        engine.sync_all(false).await.unwrap();
    }
    let first = db.active_instance_id().unwrap().unwrap();
    assert!(first.starts_with("legacy:"));
    // Second run must reuse the same legacy id.
    {
        let mut engine = SyncEngine::new(&source, &mut db, &cfg);
        engine.sync_all(false).await.unwrap();
    }
    assert_eq!(db.active_instance_id().unwrap().unwrap(), first);
}

/// Unused import guard for BTreeMap (kept for VersionMap clarity).
#[allow(dead_code)]
fn _type_check(_: BTreeMap<String, u64>) {}
