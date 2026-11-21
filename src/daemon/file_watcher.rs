use crate::daemon::engine::DaemonEngine;
use crate::models::Trigger;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::Mutex;

pub struct FileWatcherService {
    engine: Arc<Mutex<DaemonEngine>>,
    /// Last successful scan, keyed by *rule id* so two rules watching one folder
    /// each diff against their own baseline and filter.
    known_dir_states: HashMap<String, HashMap<PathBuf, (SystemTime, u64)>>,
    /// Rules whose last scan failed; used to log the failure once per episode.
    scan_failures: HashSet<String>,
}

impl FileWatcherService {
    pub fn new(engine: Arc<Mutex<DaemonEngine>>) -> Self {
        Self {
            engine,
            known_dir_states: HashMap::new(),
            scan_failures: HashSet::new(),
        }
    }

    pub async fn poll_once(&mut self) -> u64 {
        let rules = {
            let eng = self.engine.lock().await;
            eng.config.rules.clone()
        };

        let has_file_rules = rules
            .iter()
            .any(|r| r.is_enabled && matches!(r.trigger, Some(Trigger::FileWatcher { .. })));

        if !has_file_rules {
            if !self.known_dir_states.is_empty() {
                self.known_dir_states.clear();
                self.scan_failures.clear();
            }
            return 5000; // Dormant mode: sleep 5s when no file watcher rules exist
        }

        // Prune tracked state for rules that were deleted or disabled
        let active_rules: HashSet<String> = rules
            .iter()
            .filter(|r| r.is_enabled && matches!(r.trigger, Some(Trigger::FileWatcher { .. })))
            .map(|r| r.id.clone())
            .collect();
        self.known_dir_states.retain(|id, _| active_rules.contains(id));
        self.scan_failures.retain(|id| active_rules.contains(id));

        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }

            if let Some(Trigger::FileWatcher {
                folder_path,
                filter_extension,
                on_created,
                on_modified,
                on_deleted,
            }) = &rule.trigger
            {
                let target_path = PathBuf::from(folder_path);
                let prev_state = self.known_dir_states.get(&rule.id).cloned();

                // `None` means the path exists but could not be read right now
                // (sleeping drive, permissions, transient I/O). A genuinely
                // missing path is an empty state, so deletions still fire once.
                let current_state = match std::fs::metadata(&target_path) {
                    Ok(meta) if meta.is_file() => Self::scan_file(&target_path),
                    Ok(_) => Self::scan_dir(&target_path, filter_extension.as_deref()),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        Some(HashMap::new())
                    }
                    Err(_) => None,
                };

                let Some(current_state) = current_state else {
                    if self.scan_failures.insert(rule.id.clone()) {
                        crate::logging::warn(
                            "FILE_WATCHER",
                            &format!(
                                "Cannot scan folder for rule '{}'; keeping the last known state",
                                rule.name
                            ),
                        );
                    }
                    continue;
                };
                if self.scan_failures.remove(&rule.id) {
                    crate::logging::info(
                        "FILE_WATCHER",
                        &format!("Folder scan recovered for rule '{}'", rule.name),
                    );
                }

                if let Some(prev) = prev_state {
                    // 1. Created files
                    if *on_created {
                        for file in current_state.keys() {
                            if !prev.contains_key(file) {
                                self.trigger_file_event(rule, "File Created", file).await;
                            }
                        }
                    }

                    // 2. Modified files
                    if *on_modified {
                        for (file, &(mtime, size)) in &current_state {
                            if let Some(&(prev_mtime, prev_size)) = prev.get(file) {
                                if mtime != prev_mtime || size != prev_size {
                                    self.trigger_file_event(rule, "File Modified", file).await;
                                }
                            }
                        }
                    }

                    // 3. Deleted files
                    if *on_deleted {
                        for file in prev.keys() {
                            if !current_state.contains_key(file) {
                                self.trigger_file_event(rule, "File Deleted", file).await;
                            }
                        }
                    }
                }

                self.known_dir_states.insert(rule.id.clone(), current_state);
            }
        }
        1500
    }

    async fn trigger_file_event(
        &self,
        rule: &crate::models::Rule,
        event_type: &str,
        file_path: &Path,
    ) {
        let file_str = file_path.to_string_lossy().to_string();
        let file_name = file_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default()
            .to_string();
        let file_dir = file_path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut eng = self.engine.lock().await;
        let mut custom_ctx = HashMap::new();
        custom_ctx.insert("FilePath".to_string(), file_str);
        custom_ctx.insert("FileName".to_string(), file_name);
        custom_ctx.insert("FileDir".to_string(), file_dir);

        let _ = eng
            .try_execute_rule_with_context(rule, event_type, 0, custom_ctx)
            .await;
    }

    fn scan_file(file: &Path) -> Option<HashMap<PathBuf, (SystemTime, u64)>> {
        match std::fs::metadata(file) {
            Ok(meta) => {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let size = meta.len();
                let mut map = HashMap::new();
                map.insert(file.to_path_buf(), (mtime, size));
                Some(map)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(HashMap::new()),
            Err(_) => None,
        }
    }

    fn scan_dir(
        dir: &Path,
        filter_ext: Option<&str>,
    ) -> Option<HashMap<PathBuf, (SystemTime, u64)>> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return None,
        };
        let mut map = HashMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(pattern) = filter_ext {
                    let clean = pattern.trim().trim_start_matches('*');
                    let matches_ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case(clean.trim_start_matches('.')))
                        .unwrap_or(false);
                    let matches_name = path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .map(|f| {
                            f.eq_ignore_ascii_case(clean)
                                || f.to_lowercase().contains(&clean.to_lowercase())
                        })
                        .unwrap_or(false);
                    if !matches_ext && !matches_name {
                        continue;
                    }
                }

                if let Ok(meta) = entry.metadata() {
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    let size = meta.len();
                    map.insert(path, (mtime, size));
                }
            }
        }
        Some(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::models::{Rule, Trigger};
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_test_dir() -> PathBuf {
        let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "synector_file_watcher_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn file_rule(id: &str, folder: &Path, filter: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::FileWatcher {
                folder_path: folder.to_string_lossy().to_string(),
                filter_extension: Some(filter.to_string()),
                on_created: true,
                on_modified: true,
                on_deleted: true,
            }),
            ..Default::default()
        }
    }

    fn triggered_rules(engine: &DaemonEngine) -> HashSet<String> {
        engine.rule_last_triggered.keys().cloned().collect()
    }

    async fn poll(service: &mut FileWatcherService) {
        // Triggered pipelines are spawned through the Tokio runtime; give them a
        // moment to log before the assertions.
        service.poll_once().await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn test_rules_on_same_folder_keep_separate_baselines() {
        let dir = unique_test_dir();
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::fs::write(dir.join("b.log"), b"b").unwrap();

        let mut config = AppConfig::default();
        config.rules.push(file_rule("fw_txt", &dir, "*.txt"));
        config.rules.push(file_rule("fw_log", &dir, "*.log"));
        let engine = Arc::new(Mutex::new(DaemonEngine::new(config)));
        let mut service = FileWatcherService::new(Arc::clone(&engine));

        // First poll establishes one baseline per rule; the old shared-by-folder
        // baseline already fired here because each rule diffed against the other
        // rule's scan.
        poll(&mut service).await;
        {
            let eng = engine.lock().await;
            assert!(
                triggered_rules(&eng).is_empty(),
                "the first scan must only record baselines"
            );
        }

        // A new .txt must fire only for the .txt rule.
        std::fs::write(dir.join("c.txt"), b"c").unwrap();
        poll(&mut service).await;
        let triggered = {
            let eng = engine.lock().await;
            triggered_rules(&eng)
        };
        assert_eq!(triggered.len(), 1, "only the .txt rule may report the new file: {triggered:?}");
        assert!(triggered.contains("fw_txt"), "{triggered:?}");

        // A new .log must fire only for the .log rule; the existing b.log must not
        // be re-reported because the two rules no longer share a baseline.
        std::fs::write(dir.join("d.log"), b"d").unwrap();
        poll(&mut service).await;
        let triggered = {
            let eng = engine.lock().await;
            triggered_rules(&eng)
        };
        assert_eq!(triggered.len(), 2, "the .log rule must fire for d.log: {triggered:?}");
        assert!(triggered.contains("fw_log"), "{triggered:?}");
        assert!(triggered.contains("fw_txt"), "{triggered:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_dir_reports_unreadable_folder_as_none() {
        let dir = unique_test_dir();
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        assert!(FileWatcherService::scan_dir(&dir, None).is_some());

        // A path that cannot be enumerated (does not exist) must not look like an
        // empty folder: `None` keeps the previous baseline instead of deleting files.
        let missing = dir.join("gone").join("deeper");
        assert!(FileWatcherService::scan_dir(&missing, None).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_file_missing_target_is_empty_not_an_error() {
        let dir = unique_test_dir();
        let file = dir.join("missing.txt");
        let state = FileWatcherService::scan_file(&file).expect("missing file is an empty state");
        assert!(state.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
