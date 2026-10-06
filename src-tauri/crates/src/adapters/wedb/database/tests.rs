use std::path::Path;

use crate::{merkle, trailer, FileDetails, FileRecord};

use super::Database;
use crate::ports::index::EndpointState;

fn details(folder_id: i64, path: &str, image: u8, text: Option<&str>) -> FileDetails {
    let image_hash = trailer::hash_bytes(&[image]);
    let processor = text.map(|_| "fixture/v1".to_owned());
    FileDetails {
        file: FileRecord {
            id: 0,
            folder_id,
            relative_path: path.into(),
            record_hash: merkle::record_hash(&image_hash, text.zip(processor.as_deref())).unwrap(),
            image_hash,
            image_length: 1,
            text_hash: text.map(|value| trailer::hash_bytes(value.as_bytes())),
            processor,
        },
        text: text.map(str::to_owned),
    }
}

#[test]
fn inspector_searches_names_and_text_with_exact_matches_first_and_folder_scoping() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    let other = database.folders().ensure("other").unwrap();
    database
        .apply_changes(
            folder,
            &[
                details(folder, "z-coffee.png", 1, None),
                details(
                    folder,
                    "b-receipt.png",
                    2,
                    Some("COFFEE and croissant\nTotal $7.50"),
                ),
                details(folder, "a-note.png", 3, Some("cofee tomorrow")),
            ],
            &[],
            "root",
        )
        .unwrap();
    database
        .apply_changes(
            other,
            &[details(other, "coffee.png", 4, Some("private coffee"))],
            &[],
            "other-root",
        )
        .unwrap();
    let page = database
        .files()
        .list(folder, "  coffee  ", 0, true)
        .unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(
        page.files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["z-coffee.png", "b-receipt.png", "a-note.png"]
    );
    assert_eq!(
        database
            .files()
            .list(folder, "coffee", 0, false)
            .unwrap()
            .total,
        2
    );
    assert_eq!(
        database
            .files()
            .list(folder, "croissant", 0, false)
            .unwrap()
            .files[0]
            .relative_path,
        "b-receipt.png"
    );
    assert_eq!(
        database
            .files()
            .list(folder, "receipt coff", 0, true)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        database
            .files()
            .list(folder, "receipt tomorrow", 0, true)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        database
            .files()
            .list(folder, "private", 0, true)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        database.files().list(folder, "  ", 0, true).unwrap().total,
        3
    );
    assert!(matches!(
        database.files().list(999, "coffee", 0, true),
        Err(crate::Error::NotFound(_))
    ));
}

#[test]
fn inspector_fuzzy_search_handles_common_typos_unicode_and_literal_operators() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    database
        .apply_changes(
            folder,
            &[details(
                folder,
                "receipt.png",
                1,
                Some("coffee croissant café λογος\nDiscount 10%"),
            )],
            &[],
            "root",
        )
        .unwrap();
    for query in [
        "cofee",
        "cofffee",
        "xoffee",
        "cofefe",
        "reciept",
        "croiss",
        "cafe",
        "λογοσ",
        "receipt cofee",
    ] {
        assert_eq!(
            database.files().list(folder, query, 0, true).unwrap().total,
            1,
            "query: {query}"
        );
    }
    for query in [
        "cat",
        "unrelated",
        "cxxfee",
        "coffee OR unrelated",
        "\" OR \"",
        "receipt NOT coffee",
    ] {
        assert_eq!(
            database.files().list(folder, query, 0, true).unwrap().total,
            0,
            "query: {query}"
        );
    }
    assert_eq!(
        database
            .files()
            .list(folder, "cofee", 0, false)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        database
            .files()
            .list(folder, "10%", 0, false)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        database.files().list(folder, "%", 0, true).unwrap().total,
        1
    );
    assert!(matches!(
        database.files().list(folder, &"x".repeat(1025), 0, false),
        Err(crate::Error::InvalidInput(_))
    ));
    assert!(database
        .files()
        .list(folder, &"x".repeat(65), 0, true)
        .unwrap()
        .notice
        .is_some());
    assert!(database
        .files()
        .list(
            folder,
            "one two three four five six seven eight nine",
            0,
            true
        )
        .unwrap()
        .notice
        .is_some());
}

#[test]
fn inspector_text_search_counts_and_pages_all_results() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    let files: Vec<_> = (0..56)
        .map(|index| {
            details(
                folder,
                &format!("image-{index:02}.png"),
                index,
                Some("coffee receipt"),
            )
        })
        .collect();
    database.apply_changes(folder, &files, &[], "root").unwrap();
    for (query, fuzzy) in [("coffee", false), ("cofee", true)] {
        let first = database.files().list(folder, query, 0, fuzzy).unwrap();
        let next = database.files().list(folder, query, 50, fuzzy).unwrap();
        assert_eq!(
            (first.total, first.files.len(), next.total, next.files.len()),
            (56, 50, 56, 6)
        );
        assert_eq!(first.files[49].relative_path, "image-49.png");
        assert_eq!(next.files[0].relative_path, "image-50.png");
        let api = database
            .search()
            .page(query, Some(folder), 7, 7, fuzzy)
            .unwrap();
        assert_eq!((api.total, api.hits.len()), (56, 7));
        assert_eq!(
            api.hits.iter().map(|hit| hit.file.id).collect::<Vec<_>>(),
            first.files[7..14]
                .iter()
                .map(|file| file.id)
                .collect::<Vec<_>>()
        );
        if fuzzy {
            assert!(api.fuzzy_applied);
            assert!(api.hits[0].snippet.contains("[coffee]"));
        }
        assert!(database
            .files()
            .list(folder, query, usize::MAX, fuzzy)
            .unwrap()
            .files
            .is_empty());
    }
}

#[test]
fn inspector_fuzzy_search_bounds_dictionary_expansion() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    let text = (0..50_001)
        .map(|index| format!("w{index:05}"))
        .collect::<Vec<_>>()
        .join(" ");
    database
        .apply_changes(
            folder,
            &[details(folder, "dictionary.png", 1, Some(&text))],
            &[],
            "root",
        )
        .unwrap();
    let fallback = database
        .search()
        .page("w00000", Some(folder), 0, 50, true)
        .unwrap();
    assert_eq!(fallback.total, 1);
    assert!(!fallback.fuzzy_applied);
    assert!(fallback.notice.unwrap().contains("Showing exact matches"));
    assert_eq!(
        database
            .files()
            .list(folder, "w00000", 0, false)
            .unwrap()
            .total,
        1
    );
}

#[test]
fn durable_queue_returns_bounded_ready_jobs_and_preserves_deferrals_and_attempts() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    let other = database.folders().ensure("other").unwrap();
    let mut files: Vec<_> = (0..80)
        .map(|index| details(folder, &format!("image-{index:02}.png"), index, None))
        .collect();
    files.push(details(folder, "processed.png", 81, Some("coffee")));
    database.apply_changes(folder, &files, &[], "root").unwrap();
    database
        .apply_changes(
            other,
            &[details(other, "other.png", 99, None)],
            &[],
            "other",
        )
        .unwrap();
    let all = database.jobs().list(folder).unwrap();
    for (job, retry) in [(&all[0], None), (&all[1], Some(500)), (&all[2], Some(50))] {
        database
            .recovery()
            .save_failure(
                "provider",
                job,
                "fixture failure",
                2,
                retry,
                &EndpointState::default(),
            )
            .unwrap();
    }
    let processed = database
        .files()
        .by_path(folder, "processed.png")
        .unwrap()
        .file;
    database
        .jobs()
        .queue(processed.id, &processed.image_hash, true)
        .unwrap();
    let ready = database
        .jobs()
        .ready(&[folder], "provider", 100, &[], 2)
        .unwrap();
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0].file.id, processed.id);
    assert!(ready[0].force);
    assert_eq!(ready[1].file.id, all[2].file.id);
    database.jobs().defer(&ready[1], 1000).unwrap();
    let after = database
        .jobs()
        .ready(&[folder], "provider", 200, &[processed.id], 2)
        .unwrap();
    assert_eq!(after.len(), 2);
    assert!(after.iter().all(|job| job.file.id != all[0].file.id
        && job.file.id != all[1].file.id
        && job.file.id != all[2].file.id
        && job.file.folder_id == folder));
    assert_eq!(
        database
            .recovery()
            .load("provider")
            .unwrap()
            .0
            .iter()
            .find(|failure| failure.file.id == all[2].file.id)
            .unwrap()
            .attempts,
        2
    );
    assert!(database
        .jobs()
        .ready(&[folder], "provider", 1000, &[processed.id], 2)
        .unwrap()
        .iter()
        .any(|job| job.file.id == all[2].file.id));
    assert!(database
        .jobs()
        .ready(&[], "provider", 200, &[], 2)
        .unwrap()
        .is_empty());
    assert!(database
        .jobs()
        .ready(&[folder], "provider", 200, &[], 0)
        .unwrap()
        .is_empty());
    let mut stale = ready[0].clone();
    stale.file.record_hash = "changed".into();
    database.jobs().defer(&stale, 9999).unwrap();
    assert_eq!(
        database
            .jobs()
            .ready(&[folder], "provider", 200, &[], 1)
            .unwrap()[0]
            .file
            .id,
        processed.id
    );
}

#[test]
fn deferred_jobs_and_retry_attempts_survive_reopening_and_manual_retry() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("index.sqlite");
    let mut database = Database::open(&path).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    database
        .apply_changes(folder, &[details(folder, "copy.png", 1, None)], &[], "root")
        .unwrap();
    let job = database.jobs().list(folder).unwrap().remove(0);
    database
        .recovery()
        .save_failure(
            "provider",
            &job,
            "try again",
            2,
            Some(50),
            &EndpointState::default(),
        )
        .unwrap();
    database.jobs().defer(&job, 1000).unwrap();
    drop(database);
    let database = Database::open(&path).unwrap();
    assert!(database
        .jobs()
        .ready(&[folder], "provider", 100, &[], 8)
        .unwrap()
        .is_empty());
    let resumed = database
        .jobs()
        .ready(&[folder], "provider", 1000, &[], 8)
        .unwrap();
    assert_eq!(resumed.len(), 1);
    assert!(resumed[0].request_id.is_some());
    let failures = database.recovery().load("provider").unwrap().0;
    assert_eq!(failures[0].attempts, 2);
    assert_eq!(failures[0].request_id, resumed[0].request_id);
    // An explicit retry gets a new request and resets the automatic readiness delay.
    database
        .jobs()
        .queue(job.file.id, &job.file.image_hash, false)
        .unwrap();
    let retried = database
        .jobs()
        .ready(&[folder], "provider", 100, &[], 8)
        .unwrap();
    assert_eq!(retried.len(), 1);
    assert_ne!(retried[0].request_id, resumed[0].request_id);
    assert!(database.recovery().load("provider").unwrap().0.is_empty());
}

#[test]
fn scan_repositories_commit_or_roll_back_together() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder_id = database.folders().ensure("images").unwrap();
    let initial = [
        details(folder_id, "note.png", 1, Some("original coffee")),
        details(folder_id, "removed.png", 2, Some("fresh bread")),
    ];
    database
        .apply_changes(folder_id, &initial, &[], "original-root")
        .unwrap();
    let file = database
        .files()
        .by_path(folder_id, "note.png")
        .unwrap()
        .file;
    database
        .jobs()
        .queue(file.id, &file.image_hash, true)
        .unwrap();
    let job = database.jobs().list(folder_id).unwrap().remove(0);
    assert!(database
        .recovery()
        .save_failure(
            "provider",
            &job,
            "rate limited",
            1,
            Some(500),
            &EndpointState {
                retry_at_ms: Some(500),
                next_request_ms: 100,
                blocked_error: None,
            },
        )
        .unwrap());

    // Fail the last write, after file, cache, and queue changes have run.
    database
        .fail_write
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let replacement = details(folder_id, "note.png", 3, Some("replacement espresso"));
    let changed = [replacement.clone()];
    let removed = ["removed.png".into()];
    assert!(database
        .apply_changes(folder_id, &changed, &removed, "updated-root")
        .is_err());

    let snapshot = database.folders().snapshot(folder_id).unwrap();
    assert_eq!(snapshot.folder.root_hash, "original-root");
    assert_eq!(snapshot.files.len(), 2);
    let retained = database.files().get(file.id).unwrap();
    assert_eq!(retained.file.record_hash, file.record_hash);
    assert_eq!(retained.text.as_deref(), Some("original coffee"));
    assert_eq!(database.jobs().request_id(file.id).unwrap(), job.request_id);
    let (failures, endpoint) = database.recovery().load("provider").unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error, "rate limited");
    assert_eq!(endpoint.retry_at_ms, Some(500));
    assert_eq!(
        database
            .cache()
            .get(&file.image_hash, "fixture/v1")
            .unwrap()
            .as_deref(),
        Some("original coffee")
    );
    assert!(database
        .cache()
        .get(&replacement.file.image_hash, "fixture/v1")
        .unwrap()
        .is_none());
    assert_eq!(
        database.search().query("coffee", None, 10).unwrap().len(),
        1
    );
    assert_eq!(database.search().query("bread", None, 10).unwrap().len(), 1);
    assert!(database
        .search()
        .query("espresso", None, 10)
        .unwrap()
        .is_empty());

    database
        .apply_changes(folder_id, &changed, &removed, "updated-root")
        .unwrap();
    let snapshot = database.folders().snapshot(folder_id).unwrap();
    assert_eq!(snapshot.folder.root_hash, "updated-root");
    assert_eq!(snapshot.files.len(), 1);
    assert_eq!(
        database.files().get(file.id).unwrap().text,
        replacement.text
    );
    assert!(database.jobs().list(folder_id).unwrap().is_empty());
    assert!(database.recovery().load("provider").unwrap().0.is_empty());
    assert_eq!(
        database
            .cache()
            .get(&replacement.file.image_hash, "fixture/v1")
            .unwrap()
            .as_deref(),
        Some("replacement espresso")
    );
    assert!(database
        .search()
        .query("coffee", None, 10)
        .unwrap()
        .is_empty());
    assert!(database
        .search()
        .query("bread", None, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        database.search().query("espresso", None, 10).unwrap().len(),
        1
    );
}

#[test]
fn failure_and_endpoint_updates_commit_or_roll_back_together() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder_id = database.folders().ensure("images").unwrap();
    database
        .apply_changes(
            folder_id,
            &[details(folder_id, "note.png", 1, None)],
            &[],
            "root",
        )
        .unwrap();
    let job = database.jobs().list(folder_id).unwrap().remove(0);
    let initial = EndpointState {
        retry_at_ms: Some(500),
        next_request_ms: 100,
        blocked_error: None,
    };
    assert!(database
        .recovery()
        .save_failure("provider", &job, "rate limited", 1, Some(500), &initial)
        .unwrap());
    database
        .fail_write
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let updated = EndpointState {
        retry_at_ms: None,
        next_request_ms: 200,
        blocked_error: Some("authentication failed".into()),
    };
    assert!(database
        .recovery()
        .save_failure("provider", &job, "authentication failed", 2, None, &updated)
        .is_err());
    let (failures, endpoint) = database.recovery().load("provider").unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error, "rate limited");
    assert_eq!(failures[0].attempts, 1);
    assert_eq!(failures[0].retry_at_ms, Some(500));
    assert_eq!(endpoint.retry_at_ms, Some(500));
    assert_eq!(endpoint.next_request_ms, 100);
    assert!(endpoint.blocked_error.is_none());

    assert!(database
        .recovery()
        .save_failure("provider", &job, "authentication failed", 2, None, &updated)
        .unwrap());
    let (failures, endpoint) = database.recovery().load("provider").unwrap();
    assert_eq!(failures[0].error, "authentication failed");
    assert_eq!(failures[0].attempts, 2);
    assert_eq!(failures[0].retry_at_ms, None);
    assert_eq!(endpoint.retry_at_ms, None);
    assert_eq!(endpoint.next_request_ms, 200);
    assert_eq!(
        endpoint.blocked_error.as_deref(),
        Some("authentication failed")
    );
}

#[test]
fn saved_results_survive_restart_and_deferral_but_not_a_new_generation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.wedb");
    let mut db = Database::open(&path).unwrap();
    let folder = db.folders().ensure("images").unwrap();
    db.apply_changes(
        folder,
        &[details(folder, "receipt.png", 1, Some("old text"))],
        &[],
        "root",
    )
    .unwrap();
    let file = db.files().by_path(folder, "receipt.png").unwrap().file;
    db.jobs().queue(file.id, &file.image_hash, true).unwrap();
    let job = db.jobs().list(folder).unwrap().remove(0);
    assert!(db.jobs().claim(&job, "old-session", 0, 1_000_000).unwrap());
    db.jobs()
        .save_result(&job, "fresh coffee", "fixture/v2")
        .unwrap();
    db.jobs().defer(&job, 1000).unwrap();
    drop(db);
    let db = Database::open(&path).unwrap();
    assert!(db
        .jobs()
        .ready(&[folder], "provider", 999, &[], 8)
        .unwrap()
        .is_empty());
    let resumed = db
        .jobs()
        .ready(&[folder], "provider", 1000, &[], 8)
        .unwrap()
        .remove(0);
    assert!(resumed.force);
    assert_eq!(
        db.jobs()
            .saved_text(&resumed, "fixture/v2")
            .unwrap()
            .as_deref(),
        Some("fresh coffee")
    );
    assert!(db
        .jobs()
        .saved_text(&resumed, "different-model")
        .unwrap()
        .is_none());
    assert_eq!(
        db.files().get(file.id).unwrap().text.as_deref(),
        Some("old text")
    );
    db.jobs().queue(file.id, &file.image_hash, true).unwrap();
    let fresh = db.jobs().list(folder).unwrap().remove(0);
    assert_ne!(fresh.request_id, resumed.request_id);
    assert!(db
        .jobs()
        .saved_text(&fresh, "fixture/v2")
        .unwrap()
        .is_none());
}

#[test]
fn a_failed_scan_keeps_exclusions_and_can_recover() {
    use crate::settings::FolderSettings;
    let temp = tempfile::tempdir().unwrap();
    let images = temp.path().join("images");
    std::fs::create_dir(&images).unwrap();
    for name in ["keep.jpg", "excluded.jpg"] {
        std::fs::write(
            images.join(name),
            include_bytes!("../../../../tests/fixtures/pixel.jpg"),
        )
        .unwrap();
    }
    let database = Database::open(&temp.path().join("index.wedb")).unwrap();
    let fail_write = database.fail_write.clone();
    let core = crate::composition::core_with_repository(Box::new(database));
    let initial = core.scan_folder(&images).unwrap();
    core.set_folder_rules(
        &images,
        &FolderSettings {
            exclusions: vec!["excluded.jpg".into()],
            ..Default::default()
        },
    )
    .unwrap();
    fail_write.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut changed = include_bytes!("../../../../tests/fixtures/pixel.jpg").to_vec();
    changed.extend_from_slice(b"new image");
    std::fs::write(images.join("keep.jpg"), changed).unwrap();
    assert!(core.reconcile_folder(&images).is_err());
    assert!(core
        .prepare_image(initial.folder.id, "excluded.jpg")
        .is_err());
    let recovered = core.reconcile_folder(&images).unwrap();
    assert_eq!((recovered.folder.image_count, recovered.removed), (1, 1));
    assert_eq!(
        recovered.folder.root_hash,
        core.scan_folder(&images).unwrap().folder.root_hash
    );
}

#[test]
fn restart_reconciles_a_trailer_written_before_job_acknowledgement() {
    use crate::Core;
    let temp = tempfile::tempdir().unwrap();
    let images = temp.path().join("images");
    std::fs::create_dir(&images).unwrap();
    let image = images.join("receipt.jpg");
    std::fs::write(
        &image,
        include_bytes!("../../../../tests/fixtures/pixel.jpg"),
    )
    .unwrap();
    let path = temp.path().join("index.wedb");
    let core = Core::open(&path).unwrap();
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let job = core.extraction_jobs(folder).unwrap().remove(0);
    core.index
        .lock()
        .unwrap()
        .save_result(&job, "coffee saved", "fixture/v1")
        .unwrap();
    trailer::write_text(&image, &job.file.image_hash, "coffee saved", "fixture/v1").unwrap();
    drop(core);
    let core = Core::open(&path).unwrap();
    core.scan_folder(&images).unwrap();
    assert!(core.extraction_jobs(folder).unwrap().is_empty());
    assert_eq!(core.search("cofee", Some(folder), 10).unwrap().len(), 1);
    assert_eq!(
        core.cached_extraction(&job.file.image_hash, "fixture/v1")
            .unwrap()
            .as_deref(),
        Some("coffee saved")
    );
}

#[test]
fn shared_text_bodies_keep_manual_edits_independent_on_identical_images() {
    let mut db = Database::open(Path::new(":memory:")).unwrap();
    let folder = db.folders().ensure("images").unwrap();
    db.apply_changes(
        folder,
        &[
            details(folder, "a.png", 1, Some("original")),
            details(folder, "b.png", 1, Some("original")),
        ],
        &[],
        "root",
    )
    .unwrap();
    db.apply_changes(
        folder,
        &[details(folder, "a.png", 1, Some("edited"))],
        &[],
        "root2",
    )
    .unwrap();
    assert_eq!(
        db.files().by_path(folder, "b.png").unwrap().text.as_deref(),
        Some("original")
    );
    assert_eq!(
        db.files().by_path(folder, "a.png").unwrap().text.as_deref(),
        Some("edited")
    );
    db.maintenance().cleanup_cache().unwrap();
    assert_eq!(
        db.files().by_path(folder, "b.png").unwrap().text.as_deref(),
        Some("original")
    );
}

#[cfg(feature = "legacy-sqlite")]
#[test]
fn legacy_sqlite_import_preserves_ids_text_cache_jobs_recovery_and_the_original() {
    use rusqlite::params;
    for version in [1, 4] {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("index.sqlite");
        let connection = rusqlite::Connection::open(&legacy).unwrap();
        connection
            .execute_batch(include_str!("../../../../migrations/001-index.sql"))
            .unwrap();
        if version == 4 {
            crate::adapters::wedb::migrations::apply(&connection).unwrap();
        }
        let file = details(17, "receipt.png", 1, Some("migration coffee"));
        connection
            .execute(
                "INSERT INTO folders(id,path,root_hash) VALUES(17,'images','root')",
                [],
            )
            .unwrap();
        connection.execute("INSERT INTO files(id,folder_id,relative_path,image_hash,image_length,text_hash,record_hash,processor,text) VALUES(42,17,?1,?2,1,?3,?4,?5,?6)",params![file.file.relative_path,file.file.image_hash,file.file.text_hash,file.file.record_hash,file.file.processor,file.text]).unwrap();
        if version == 4 {
            connection
                .execute(
                    "INSERT INTO extraction_cache VALUES(?1,'fixture/v1','migration coffee')",
                    [&file.file.image_hash],
                )
                .unwrap();
            connection.execute("INSERT INTO extraction_jobs(id,file_id,image_hash,force,ready_at_ms) VALUES(81,42,?1,1,1000)",[&file.file.image_hash]).unwrap();
            connection
                .execute(
                    "INSERT INTO extraction_failures VALUES(42,'provider',?1,81,'retry me',2,1200)",
                    [&file.file.image_hash],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO extraction_endpoints VALUES('provider',1200,1500,NULL)",
                    [],
                )
                .unwrap();
        }
        drop(connection);
        let before = std::fs::read(&legacy).unwrap();
        let db = Database::open(&legacy).unwrap();
        assert_eq!(
            db.files().get(42).unwrap().text.as_deref(),
            Some("migration coffee")
        );
        assert_eq!(
            db.cache()
                .get(&file.file.image_hash, "fixture/v1")
                .unwrap()
                .as_deref(),
            Some("migration coffee")
        );
        assert_eq!(db.search().query("cofee", None, 10).unwrap().len(), 1);
        if version == 4 {
            let job = db.jobs().list(17).unwrap().remove(0);
            assert_eq!(job.request_id, Some(81));
            assert!(job.force);
            assert!(db
                .jobs()
                .ready(&[17], "provider", 1199, &[], 8)
                .unwrap()
                .is_empty());
            assert_eq!(
                db.jobs()
                    .ready(&[17], "provider", 1200, &[], 8)
                    .unwrap()
                    .len(),
                1
            );
            let (failures, endpoint) = db.recovery().load("provider").unwrap();
            assert_eq!(failures[0].attempts, 2);
            assert_eq!(endpoint.next_request_ms, 1500);
        } else {
            assert!(db.jobs().list(17).unwrap().is_empty());
        }
        assert_eq!(before, std::fs::read(&legacy).unwrap());
        drop(db);
        assert_eq!(
            Database::open(&legacy)
                .unwrap()
                .files()
                .get(42)
                .unwrap()
                .text
                .as_deref(),
            Some("migration coffee")
        );
    }
}

#[test]
fn crash_recovery_restores_synced_records_and_reclaims_abandoned_work() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("crash.wedb");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "adapters::wedb::database::tests::crash_writer_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("LENSCRIBE_CRASH_DB", &path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "child exited before syncing"
        );
        if line.contains("SYNCED") {
            break;
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let db = Database::open(&path).unwrap();
    let folder = db.folders().list().unwrap().remove(0).id;
    let job = db
        .jobs()
        .ready(&[folder], "provider", 0, &[], 8)
        .unwrap()
        .remove(0);
    assert_eq!(
        db.jobs().saved_text(&job, "fixture/v1").unwrap().as_deref(),
        Some("durable response")
    );
    assert_eq!(db.folders().get(folder).unwrap().root_hash, "root");
    assert!(db.jobs().claim(&job, "new-session", 0, 2000).unwrap());
    assert!(!db.jobs().claim(&job, "another-session", 0, 2000).unwrap());
}

#[cfg(feature = "legacy-sqlite")]
#[test]
fn future_sqlite_schema_is_rejected_without_modifying_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA user_version=99; CREATE TABLE keep(value TEXT); INSERT INTO keep VALUES('untouched');").unwrap();
    drop(connection);
    let original = std::fs::read(&path).unwrap();
    assert!(Database::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let store = super::store::Store::open(&path.with_extension("wedb")).unwrap();
    assert!(store.get::<u32>("schema").unwrap().is_none());
}

#[test]
fn queue_rechecks_backoff_when_the_clock_moves_backwards() {
    let mut database = Database::open(Path::new(":memory:")).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    database
        .apply_changes(
            folder,
            &[details(folder, "receipt.png", 1, None)],
            &[],
            "root",
        )
        .unwrap();
    let job = database.jobs().list(folder).unwrap().remove(0);
    database.jobs().defer(&job, 1000).unwrap();
    assert_eq!(
        database
            .jobs()
            .ready(&[folder], "provider", 1000, &[], 8)
            .unwrap()
            .len(),
        1
    );
    assert!(database
        .jobs()
        .ready(&[folder], "provider", 900, &[], 8)
        .unwrap()
        .is_empty());
    assert_eq!(
        database
            .jobs()
            .next_due(&[folder], "provider", &[])
            .unwrap(),
        Some(1000)
    );
}

#[test]
fn uncertain_sync_failure_blocks_writes_and_reopening_rebuilds_projections() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.wedb");
    let mut database = Database::open(&path).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    database.store.fail_sync.set(true);
    assert!(database
        .apply_changes(
            folder,
            &[details(folder, "receipt.png", 1, Some("coffee"))],
            &[],
            "root"
        )
        .is_err());
    assert_eq!(database.folders().get(folder).unwrap().image_count, 0);
    assert!(database
        .search()
        .query("coffee", None, 10)
        .unwrap()
        .is_empty());
    assert!(database.persist().is_err());
    assert!(database.folders().ensure("other").is_err());
    // The atomic engine batch may already have committed despite an uncertain
    // sync result. Reopening must rebuild projections rather than reuse stale ones.
    database.store.db.persist().unwrap();
    drop(database);
    let database = Database::open(&path).unwrap();
    assert_eq!(database.folders().get(folder).unwrap().root_hash, "root");
    assert_eq!(database.folders().get(folder).unwrap().image_count, 1);
    assert_eq!(
        database.search().query("coffee", None, 10).unwrap().len(),
        1
    );
}

#[test]
fn persisted_merkle_checkpoints_round_trip_and_invalid_checkpoints_are_disposable() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.wedb");
    let mut database = Database::open(&path).unwrap();
    let folder = database.folders().ensure("images").unwrap();
    let file = details(folder, "receipts/coffee.png", 1, Some("coffee"));
    let mut tree = merkle::MerkleTree::new();
    tree.insert(&file.file.relative_path, &file.file.record_hash)
        .unwrap();
    database
        .apply_scan(folder, &[file], &[], &tree.root_hash(), Some(&tree))
        .unwrap();
    drop(database);
    let database = Database::open(&path).unwrap();
    let files = database.files().in_folder(folder).unwrap();
    let restored = database
        .merkle_checkpoint(folder, &files, &tree.root_hash())
        .unwrap()
        .unwrap();
    assert_eq!(restored.root_hash(), tree.root_hash());
    database
        .commit(vec![super::store::Change::put(
            format!("merkle/{folder:020}/disconnected"),
            &std::collections::BTreeMap::<String, merkle::Child>::new(),
        )
        .unwrap()])
        .unwrap();
    assert!(database
        .merkle_checkpoint(folder, &files, &tree.root_hash())
        .unwrap()
        .is_none());
    assert_eq!(
        database.search().query("coffee", None, 10).unwrap().len(),
        1
    );
}

#[test]
#[ignore = "subprocess fixture, invoked by crash recovery test"]
fn crash_writer_fixture() {
    use std::io::Write;
    let Some(path) = std::env::var_os("LENSCRIBE_CRASH_DB") else {
        return;
    };
    let mut db = Database::open(Path::new(&path)).unwrap();
    let folder = db.folders().ensure("images").unwrap();
    db.apply_changes(
        folder,
        &[details(folder, "receipt.png", 1, None)],
        &[],
        "root",
    )
    .unwrap();
    let job = db.jobs().list(folder).unwrap().remove(0);
    db.jobs().claim(&job, "dead-session", 0, 1000000).unwrap();
    db.jobs()
        .save_result(&job, "durable response", "fixture/v1")
        .unwrap();
    println!("SYNCED");
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
#[ignore = "manual storage/search scale probe"]
fn storage_scaling_probe() {
    use std::time::Instant;
    let count = std::env::var("LENSCRIBE_BENCH_FILES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10000);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("probe.wedb");
    let mut db = Database::open(&path).unwrap();
    let folder = db.folders().ensure("images").unwrap();
    let files=(0..count).map(|index| {
        let mut file=details(folder,&format!("image-{index:010}.png"),0,Some("Coffee receipt. Croissant and espresso. Paid by card. Total 12.50. Store address and purchase details."));
        file.file.image_hash=trailer::hash_bytes(&index.to_le_bytes());
        file.file.record_hash=merkle::record_hash(&file.file.image_hash,file.text.as_deref().zip(file.file.processor.as_deref())).unwrap();file
    }).collect::<Vec<_>>();
    let start = Instant::now();
    db.apply_changes(folder, &files, &[], "root").unwrap();
    let write = start.elapsed();
    let start = Instant::now();
    let exact = db
        .search()
        .page("coffee", Some(folder), 0, 50, false)
        .unwrap();
    let literal = start.elapsed();
    assert_eq!(exact.total, count);
    let start = Instant::now();
    let fuzzy = db
        .search()
        .page("cofee", Some(folder), 0, 50, true)
        .unwrap();
    let fuzzy_time = start.elapsed();
    assert_eq!(fuzzy.total, count);
    assert!(fuzzy.notice.is_none());
    let path2 = temp.path().join("pending.wedb");
    let mut pending = Database::open(&path2).unwrap();
    let pf = pending.folders().ensure("images").unwrap();
    let pending_files = files
        .iter()
        .map(|file| {
            let mut file = file.clone();
            file.file.folder_id = pf;
            file.file.text_hash = None;
            file.file.processor = None;
            file.text = None;
            file.file.record_hash = merkle::record_hash(&file.file.image_hash, None).unwrap();
            file
        })
        .collect::<Vec<_>>();
    pending
        .apply_changes(pf, &pending_files, &[], "root")
        .unwrap();
    pending.jobs().ready(&[pf], "provider", 0, &[], 8).unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        assert_eq!(
            pending
                .jobs()
                .ready(&[pf], "provider", 0, &[], 8)
                .unwrap()
                .len(),
            8
        );
    }
    let queue = start.elapsed() / 100;
    drop(pending);
    drop(pending_files);
    drop(db);
    let start = Instant::now();
    let reopened = Database::open(&path).unwrap();
    let reopen = start.elapsed();
    assert_eq!(reopened.folders().get(folder).unwrap().image_count, count);
    println!("WeDB files={count} batch_ms={} literal_page_ms={} fuzzy_page_ms={} ready_8_us={} reopen_ms={}",write.as_millis(),literal.as_millis(),fuzzy_time.as_millis(),queue.as_micros(),reopen.as_millis());
    #[cfg(feature = "legacy-sqlite")]
    {
        let mut connection =
            rusqlite::Connection::open(temp.path().join("baseline.sqlite")).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        crate::adapters::wedb::migrations::apply(&connection).unwrap();
        let start = Instant::now();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO folders(id,path,root_hash) VALUES(1,'images','root')",
                [],
            )
            .unwrap();
        {
            let mut insert=transaction.prepare_cached("INSERT INTO files(folder_id,relative_path,image_hash,image_length,text_hash,record_hash,processor,text) VALUES(1,?1,?2,1,?3,?4,?5,?6)").unwrap();
            let mut cache = transaction
                .prepare_cached("INSERT INTO extraction_cache VALUES(?1,?2,?3)")
                .unwrap();
            for file in &files {
                insert
                    .execute(rusqlite::params![
                        file.file.relative_path,
                        file.file.image_hash,
                        file.file.text_hash,
                        file.file.record_hash,
                        file.file.processor,
                        file.text
                    ])
                    .unwrap();
                cache
                    .execute(rusqlite::params![
                        file.file.image_hash,
                        file.file.processor,
                        file.text
                    ])
                    .unwrap();
            }
        }
        transaction.commit().unwrap();
        let write = start.elapsed();
        let start = Instant::now();
        let total: i64 = connection
            .query_row(
                "SELECT count(*) FROM files WHERE instr(lower(text),'coffee')>0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, count as i64);
        let literal = start.elapsed();
        println!("SQLite files={count} batch_ms={} literal_count_ms={} (count only; not fuzzy or snippet parity)",write.as_millis(),literal.as_millis());
    }
}
