use std::path::Path;

use crate::{merkle, trailer, FileDetails, FileRecord};

use super::{Database, EndpointState};

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
        .connection
        .execute_batch(
            "CREATE TRIGGER reject_root BEFORE UPDATE ON folders
             BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
        )
        .unwrap();
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
        .connection
        .execute_batch("DROP TRIGGER reject_root;")
        .unwrap();
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
        .connection
        .execute_batch(
            "CREATE TRIGGER reject_endpoint BEFORE UPDATE ON extraction_endpoints
             BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;",
        )
        .unwrap();
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

    database
        .connection
        .execute_batch("DROP TRIGGER reject_endpoint;")
        .unwrap();
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
