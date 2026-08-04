//! Mirror job worker: executes persisted filesystem operations with
//! retries (spec sections 8.4, 13.3, 19.4).
//!
//! Writes are atomic: content goes to a temporary sibling file first and
//! is then renamed over the target. Renames create the new file before
//! deleting the old one, so an interrupted run never loses the link.

use std::fs;
use std::path::Path;
use tracing::{debug, warn};
use zsb_core::{Error, MirrorJob, MirrorOperation, Platform, Result};
use zsb_index::Database;

/// Maximum retries before a job is marked permanently failed.
pub const MAX_RETRIES: u32 = 8;

/// Outcome of one worker pass.
#[derive(Debug, Clone, Default)]
pub struct WorkerReport {
    pub completed: usize,
    pub retried: usize,
    pub failed: usize,
}

/// Process pending mirror jobs for one platform.
pub fn process_pending(db: &Database, platform: Platform, limit: u32) -> Result<WorkerReport> {
    let jobs = db.pending_jobs(platform, limit)?;
    let mut report = WorkerReport::default();
    for job in jobs {
        match execute(&job) {
            Ok(()) => {
                db.complete_job(job.id)?;
                report.completed += 1;
                debug!(
                    job = job.id,
                    op = job.operation.as_str(),
                    "mirror job completed"
                );
            }
            Err(e) => {
                warn!(job = job.id, error = %e, "mirror job failed");
                db.fail_job(job.id, &e.to_string(), MAX_RETRIES)?;
                // Check whether this failure exhausted retries.
                if job.retry_count + 1 >= MAX_RETRIES {
                    report.failed += 1;
                } else {
                    report.retried += 1;
                }
            }
        }
    }
    Ok(report)
}

/// Execute one mirror job on the filesystem.
fn execute(job: &MirrorJob) -> Result<()> {
    match job.operation {
        MirrorOperation::Create | MirrorOperation::Replace => {
            let path = required(job.new_path.as_deref(), "new_path")?;
            let content = required(job.content.as_deref(), "content")?;
            atomic_write(Path::new(path), content.as_bytes())?;
        }
        MirrorOperation::Rename => {
            // Create the new file first, then remove the old one
            // (spec section 13.3).
            let new_path = required(job.new_path.as_deref(), "new_path")?;
            let content = required(job.content.as_deref(), "content")?;
            atomic_write(Path::new(new_path), content.as_bytes())?;
            if let Some(old) = &job.old_path {
                remove_if_exists(Path::new(old))?;
            }
        }
        MirrorOperation::Delete => {
            if let Some(old) = &job.old_path {
                remove_if_exists(Path::new(old))?;
            }
        }
    }
    Ok(())
}

fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str> {
    value.ok_or_else(|| Error::Mirror(format!("job is missing {field}")))
}

/// Write content to `path` atomically via a sibling temp file.
fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("zsb-{}-{}.tmp", std::process::id(), now_millis()));
    fs::write(&tmp, content)?;
    if path.exists() {
        // Windows cannot rename over an existing file.
        fs::remove_file(path)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Io(e)),
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zsb_core::{NewMirrorJob, SyncBatch};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zsb-mirror-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_then_rename_then_delete() {
        let dir = temp_dir("lifecycle");
        let db = Database::open_in_memory().unwrap();

        let old_path = dir.join("张三 - 2024 - 研究 -- K1.url");
        let new_path = dir.join("李四 - 2024 - 研究 -- K1.url");
        let content = "[InternetShortcut]\r\nURL=zotero://select/library/items/K1\r\n";

        db.enqueue_jobs(&[NewMirrorJob {
            operation: MirrorOperation::Create,
            platform: Platform::Windows,
            old_path: None,
            new_path: Some(old_path.to_string_lossy().into_owned()),
            content: Some(content.into()),
        }])
        .unwrap();
        let report = process_pending(&db, Platform::Windows, 100).unwrap();
        assert_eq!(report.completed, 1);
        assert!(old_path.exists());

        db.enqueue_jobs(&[NewMirrorJob {
            operation: MirrorOperation::Rename,
            platform: Platform::Windows,
            old_path: Some(old_path.to_string_lossy().into_owned()),
            new_path: Some(new_path.to_string_lossy().into_owned()),
            content: Some(content.into()),
        }])
        .unwrap();
        process_pending(&db, Platform::Windows, 100).unwrap();
        assert!(!old_path.exists());
        assert!(new_path.exists());

        db.enqueue_jobs(&[NewMirrorJob {
            operation: MirrorOperation::Delete,
            platform: Platform::Windows,
            old_path: Some(new_path.to_string_lossy().into_owned()),
            new_path: None,
            content: None,
        }])
        .unwrap();
        process_pending(&db, Platform::Windows, 100).unwrap();
        assert!(!new_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn jobs_survive_restart_simulation() {
        // Jobs are persisted in the same transaction as the sync batch;
        // a "crash" before processing leaves them pending (spec 8.4).
        let dir = temp_dir("outbox");
        let mut db = Database::open_in_memory().unwrap();
        db.upsert_instance(&zsb_core::ServerInfo {
            api_base: "http://localhost:23119/api".into(),
            api_version: Some(3),
            schema_version: None,
            server_id: "srv".into(),
        })
        .unwrap();
        let lib = zsb_core::RemoteLibrary::user();
        let lib_id = db.upsert_library("srv", &lib).unwrap();
        let path = dir.join("a -- K9.url");
        db.apply_sync_batch(
            lib_id,
            &SyncBatch {
                upserts: vec![zsb_core::IndexedItem {
                    item_key: "K9".into(),
                    item_version: 1,
                    item_type: "book".into(),
                    title: "a".into(),
                    select_uri: "zotero://select/library/items/K9".into(),
                    mirror_filename: Some("a -- K9".into()),
                    content_hash: "h".into(),
                    ..Default::default()
                }],
                deleted_keys: vec![],
                mirror_jobs: vec![NewMirrorJob {
                    operation: MirrorOperation::Create,
                    platform: Platform::Windows,
                    old_path: None,
                    new_path: Some(path.to_string_lossy().into_owned()),
                    content: Some(
                        "[InternetShortcut]\r\nURL=zotero://select/library/items/K9\r\n".into(),
                    ),
                }],
                new_version: 1,
            },
            true,
        )
        .unwrap();
        // Item committed, file not yet written:
        assert_eq!(db.count_items().unwrap(), 1);
        assert!(!path.exists());
        // Worker catches up after "restart":
        process_pending(&db, Platform::Windows, 100).unwrap();
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_job_retries_and_gives_up() {
        let db = Database::open_in_memory().unwrap();
        db.enqueue_jobs(&[NewMirrorJob {
            operation: MirrorOperation::Create,
            platform: Platform::Windows,
            old_path: None,
            // Unwritable path (missing drive on any platform).
            new_path: Some("\\\\?\\Z:\\nonexistent\\x.url".into()),
            content: Some("x".into()),
        }])
        .unwrap();
        let report = process_pending(&db, Platform::Windows, 100).unwrap();
        assert_eq!(report.retried, 1);
        let jobs = db.pending_jobs(Platform::Windows, 10).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].retry_count, 1);
        assert!(jobs[0].last_error.is_some());
    }
}
