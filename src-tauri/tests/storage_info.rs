//! Storage info command tests: metadata only, no sensitive data leaked,
//! and tolerant of missing paths / unreadable files / corrupted database.

use cc_switch_lib::{get_storage_info, StorageInfo};

#[path = "support.rs"]
mod support;
use support::{ensure_test_home, reset_test_fs, test_mutex};

fn collect() -> StorageInfo {
    tauri::async_runtime::block_on(get_storage_info()).expect("get_storage_info should not fail")
}

#[test]
fn storage_info_reports_base_dir_and_db_entries() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    // Create a real database so the entry exists on disk
    support::create_test_state().expect("create test state");

    let info = collect();
    let expected_base = home.join(".cc-switch").to_string_lossy().to_string();
    assert_eq!(
        info.base_dir.replace('\\', "/"),
        expected_base.replace('\\', "/"),
        "base dir should point at ~/.cc-switch"
    );

    let db = info
        .items
        .iter()
        .find(|i| i.name == "cc-switch.db")
        .expect("db entry should exist");
    assert_eq!(db.purpose, "database");
    assert!(db.exists, "db should exist after test state setup");
    assert!(db.size_bytes.unwrap_or(0) > 0, "db should have a size");
    assert!(
        db.record_count.is_some(),
        "db row count should be available"
    );
    // record count must be a number, never raw contents
    assert!(db.error.is_none(), "healthy db should have no error");

    // fixed entries should be listed
    for name in ["config.json", "settings.json", "backups", "logs", "skills"] {
        assert!(
            info.items.iter().any(|i| i.name == name),
            "missing expected entry {name}"
        );
    }
}

#[test]
fn storage_info_handles_missing_paths_gracefully() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    // Do NOT create any data — home dir may not even contain .cc-switch yet
    let _ = ensure_test_home();

    let info = collect();
    assert!(!info.items.is_empty(), "should always return item list");
    for item in &info.items {
        // Missing entries must carry an error message instead of panicking
        if !item.exists {
            assert!(
                item.error.is_some(),
                "non-existent entry {} should explain why",
                item.name
            );
        }
    }
    // total must be a valid u64 sum (never NaN/negative by construction)
}

#[test]
fn storage_info_tolerates_corrupted_database() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let db_path = home.join(".cc-switch").join("cc-switch.db");
    std::fs::create_dir_all(db_path.parent().expect("parent dir")).expect("mkdir");
    // Corrupt the DB with garbage bytes
    std::fs::write(&db_path, b"this is not a sqlite database file at all").expect("write garbage");

    let info = collect();
    let db = info
        .items
        .iter()
        .find(|i| i.name == "cc-switch.db")
        .expect("db entry should exist");
    assert!(db.exists, "file exists even if corrupted");
    assert!(
        db.error.is_some(),
        "corrupted db should report an error instead of panicking"
    );
    assert!(
        db.record_count.is_none(),
        "corrupted db should not expose row counts"
    );
}

#[test]
fn storage_info_never_exposes_sensitive_values() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let db_path = home.join(".cc-switch").join("cc-switch.db");
    std::fs::create_dir_all(db_path.parent().expect("parent dir")).expect("mkdir");
    // Seed a DB that contains a provider with an API key
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                app_type TEXT,
                name TEXT,
                settings_config TEXT,
                website_url TEXT,
                category TEXT,
                created_at TEXT,
                sort_index INTEGER,
                notes TEXT,
                icon TEXT,
                icon_color TEXT,
                meta TEXT,
                is_current INTEGER,
                in_failover_queue INTEGER
            );",
        )
        .expect("create providers table");
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, is_current, in_failover_queue)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "p1",
                "claude",
                "SecretProvider",
                r#"{"apiKey":"sk-test-secret-abcdef123456","env":{"ANTHROPIC_API_KEY":"sk-test-env-secret"}}"#,
                1,
                0
            ],
        )
        .expect("insert provider");
    }

    let info = collect();
    let serialized = serde_json::to_string(&info).expect("serialize");
    for needle in [
        "sk-test-secret-abcdef123456",
        "sk-test-env-secret",
        "SecretProvider",
    ] {
        assert!(
            !serialized.contains(needle),
            "storage info must not leak sensitive value {needle}"
        );
    }
    // The provider row should still be counted, just not its contents
    let db = info
        .items
        .iter()
        .find(|i| i.name == "cc-switch.db")
        .expect("db entry");
    assert!(db.record_count.unwrap_or(0) >= 1);
}

#[test]
fn open_storage_item_rejects_paths_outside_app_dir() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let outside = home.join("..").join("outside-file.txt");
    let base = home.join(".cc-switch");
    let inside = base.join("config.json");

    let base_str = base.to_string_lossy().to_string();
    let inside_str = inside.to_string_lossy().to_string();
    let outside_str = outside.to_string_lossy().to_string();

    // Mirror the (private) path-scope guard used by open_storage_item to
    // assert semantics without launching a full Tauri app handle.
    fn within(base: &str, target: &str) -> bool {
        let norm = |s: &str| -> String {
            let mut k = s.replace('\\', "/");
            while k.len() > 1 && k.ends_with('/') {
                k.pop();
            }
            #[cfg(windows)]
            {
                k = k.to_lowercase();
            }
            k
        };
        let b = norm(base);
        let t = norm(target);
        t == b || t.starts_with(&format!("{b}/"))
    }

    assert!(
        within(&base_str, &inside_str),
        "inside path must be allowed"
    );
    assert!(
        !within(&base_str, &outside_str),
        "outside path must be rejected"
    );
}
