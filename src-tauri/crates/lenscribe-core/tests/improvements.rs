use lenscribe_core::{settings::FolderSettings, trailer, Core, Error};
use std::{fs, path::Path, sync::Arc};
use tempfile::tempdir;

fn png(path: &Path, color: [u8; 3]) {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&color)
            .unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn targeted_changes_hash_only_affected_files_and_match_full_merkle_rebuild() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir_all(root.join("notes")).unwrap();
    png(&root.join("a.png"), [1, 2, 3]);
    png(&root.join("b.png"), [4, 5, 6]);
    png(&root.join("notes/c.png"), [7, 8, 9]);
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let initial = core.scan_folder(&root).unwrap();
    assert_eq!(initial.inspected, 3);
    assert_eq!(core.reconcile_folder(&root).unwrap().inspected, 0);
    png(&root.join("b.png"), [9, 8, 7]);
    let report = core
        .scan_paths(initial.folder.id, &[root.join("b.png")])
        .unwrap();
    assert_eq!(
        (report.inspected, report.changed, report.removed),
        (1, 1, 0)
    );
    assert_ne!(initial.folder.root_hash, report.folder.root_hash);
    assert_eq!(
        report.folder.root_hash,
        core.scan_folder(&root).unwrap().folder.root_hash
    );
    assert_eq!(core.reconcile_folder(&root).unwrap().inspected, 0);
    let file = core
        .snapshot(initial.folder.id)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.relative_path == "a.png")
        .unwrap();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "receipt coffee",
        "fixture/v1",
    )
    .unwrap();
    assert_eq!(core.reconcile_folder(&root).unwrap().inspected, 0);
    assert_eq!(core.search("coffee", None, 10).unwrap().len(), 1);
}

#[test]
fn parent_directory_events_do_not_rehash_unrelated_images_or_hide_file_events() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir_all(root.join("nested")).unwrap();
    png(&root.join("nested/changed.png"), [1, 2, 3]);
    png(&root.join("nested/unchanged.png"), [4, 5, 6]);
    png(&root.join("root.png"), [7, 8, 9]);
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let first = core.scan_folder(&root).unwrap();
    let path = root.join("nested/changed.png");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let before_length = fs::metadata(&path).unwrap().len();
    png(&path, [9, 8, 7]);
    assert_eq!(fs::metadata(&path).unwrap().len(), before_length);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let report = core
        .scan_paths(first.folder.id, &[root.clone(), root.join("nested"), path])
        .unwrap();
    assert_eq!((report.inspected, report.changed), (1, 1));
    assert_ne!(report.folder.root_hash, first.folder.root_hash);
    assert_eq!(
        report.folder.root_hash,
        core.scan_folder(root).unwrap().folder.root_hash
    );
}

#[test]
fn directory_renames_and_deletions_reconcile_the_subtree_and_prune_search() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir_all(root.join("old/deep")).unwrap();
    png(&root.join("old/deep/note.png"), [1, 2, 3]);
    png(&root.join("keep.png"), [4, 5, 6]);
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let first = core.scan_folder(&root).unwrap();
    let file = core
        .snapshot(first.folder.id)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.relative_path.contains("note"))
        .unwrap();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "renamed coffee",
        "fixture/v1",
    )
    .unwrap();
    fs::rename(root.join("old"), root.join("new")).unwrap();
    let changed = core
        .scan_paths(
            first.folder.id,
            &[
                root.join("old"),
                root.join("new"),
                root.join("new/deep/note.png"),
            ],
        )
        .unwrap();
    assert_eq!(
        (changed.inspected, changed.changed, changed.removed),
        (1, 1, 1)
    );
    assert_eq!(
        core.search("coffee", None, 10).unwrap()[0]
            .file
            .relative_path,
        "new/deep/note.png"
    );
    assert_eq!(
        changed.folder.root_hash,
        core.scan_folder(&root).unwrap().folder.root_hash
    );
    fs::remove_dir_all(root.join("new")).unwrap();
    let removed = core
        .scan_paths(first.folder.id, &[root.join("new")])
        .unwrap();
    assert_eq!((removed.inspected, removed.removed), (0, 1));
    assert!(core.search("coffee", None, 10).unwrap().is_empty());
    assert_eq!(
        removed.folder.root_hash,
        core.scan_folder(&root).unwrap().folder.root_hash
    );
}

#[test]
#[cfg(any(windows, unix))]
fn aliased_event_paths_stay_scoped_for_changes_renames_and_deletions() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir_all(root.join("old/deep")).unwrap();
    png(&root.join("old/deep/note.png"), [1, 2, 3]);
    png(&root.join("keep.png"), [4, 5, 6]);
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let first = core.scan_folder(&root).unwrap();
    #[cfg(windows)]
    let alias = std::path::PathBuf::from(root.to_string_lossy().to_ascii_lowercase());
    #[cfg(unix)]
    let alias = {
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        alias
    };
    let path = root.join("old/deep/note.png");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    png(&path, [9, 8, 7]);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let changed = core
        .scan_paths(first.folder.id, &[alias.join("old/deep/note.png")])
        .unwrap();
    assert_eq!(
        (changed.inspected, changed.changed, changed.removed),
        (1, 1, 0)
    );
    fs::rename(root.join("old"), root.join("new")).unwrap();
    let renamed = core
        .scan_paths(first.folder.id, &[alias.join("old"), alias.join("new")])
        .unwrap();
    assert_eq!(
        (renamed.inspected, renamed.changed, renamed.removed),
        (1, 1, 1)
    );
    assert_eq!(
        renamed.folder.root_hash,
        core.scan_folder(&root).unwrap().folder.root_hash
    );
    fs::remove_dir_all(root.join("new")).unwrap();
    let removed = core
        .scan_paths(first.folder.id, &[alias.join("new/deep/note.png")])
        .unwrap();
    assert_eq!(
        (removed.inspected, removed.changed, removed.removed),
        (0, 0, 1)
    );
    assert_eq!(
        removed.folder.root_hash,
        core.scan_folder(&root).unwrap().folder.root_hash
    );
}

#[test]
#[cfg(unix)]
fn aliased_root_events_still_prune_symlinked_subtrees_inside_the_folder() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir_all(root.join("old")).unwrap();
    fs::create_dir_all(root.join("keep")).unwrap();
    png(&root.join("old/note.png"), [1, 2, 3]);
    png(&root.join("keep/note.png"), [4, 5, 6]);
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let first = core.scan_folder(&root).unwrap();
    fs::remove_dir_all(root.join("old")).unwrap();
    std::os::unix::fs::symlink(root.join("keep"), root.join("old")).unwrap();
    let report = core
        .scan_paths(first.folder.id, &[alias.join("old")])
        .unwrap();
    assert_eq!(
        (report.inspected, report.changed, report.removed),
        (0, 0, 1)
    );
    assert_eq!(
        report.folder.root_hash,
        core.scan_folder(&root).unwrap().folder.root_hash
    );
}

#[test]
fn exclusions_prune_the_index_and_queue_without_changing_image_text() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir_all(root.join("nested/temp")).unwrap();
    png(&root.join("keep.png"), [1, 2, 3]);
    png(&root.join("nested/temp/note.png"), [4, 5, 6]);
    png(&root.join("nested/pic-thumbnail.png"), [7, 8, 9]);
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let first = core.scan_folder(&root).unwrap();
    let file = core
        .snapshot(first.folder.id)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.relative_path.contains("note"))
        .unwrap();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "preserved coffee",
        "fixture/v1",
    )
    .unwrap();
    let original = fs::read(root.join(&file.relative_path)).unwrap();
    core.queue_file(file.id, &file.image_hash, true).unwrap();
    let rules = FolderSettings {
        exclusions: vec!["temp/".into(), "*-thumbnail.png".into()],
        ..FolderSettings::default()
    };
    core.set_folder_rules(&root, &rules).unwrap();
    assert_eq!(core.reconcile_folder(&root).unwrap().removed, 2);
    assert_eq!(core.snapshot(first.folder.id).unwrap().files.len(), 1);
    assert_eq!(fs::read(root.join(&file.relative_path)).unwrap(), original);
    assert!(core.search("coffee", None, 10).unwrap().is_empty());
    assert!(core
        .prepare_image(file.folder_id, &file.relative_path)
        .is_err());
    assert_eq!(core.extraction_jobs(first.folder.id).unwrap().len(), 1); // only the included pending image
    core.set_folder_rules(&root, &FolderSettings::default())
        .unwrap();
    assert_eq!(core.reconcile_folder(&root).unwrap().changed, 2);
    assert_eq!(core.search("coffee", None, 10).unwrap().len(), 1);
}

#[test]
fn size_rules_measure_original_bytes_and_invalid_patterns_are_rejected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    png(&root.join("small.png"), [1, 2, 3]);
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let first = core.scan_folder(&root).unwrap();
    let file = core.snapshot(first.folder.id).unwrap().files[0].clone();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        &"x".repeat(2 * 1024 * 1024),
        "fixture/v1",
    )
    .unwrap();
    let mut bytes =
        fs::read(root.join("small.png")).unwrap()[..file.image_length as usize].to_vec();
    bytes.resize(2 * 1024 * 1024, 0);
    fs::write(root.join("large.png"), bytes).unwrap();
    core.set_folder_rules(
        &root,
        &FolderSettings {
            max_image_mib: 1,
            ..FolderSettings::default()
        },
    )
    .unwrap();
    let report = core.reconcile_folder(&root).unwrap();
    assert_eq!(report.folder.image_count, 1);
    assert_eq!(
        core.file(file.id).unwrap().text.unwrap().len(),
        2 * 1024 * 1024
    );
    assert!(core
        .set_folder_rules(
            &root,
            &FolderSettings {
                exclusions: vec!["[bad".into()],
                ..FolderSettings::default()
            }
        )
        .is_err());
}

#[test]
fn cached_extractions_survive_removal_restart_and_preserve_empty_text() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    png(&root.join("empty.png"), [1, 2, 3]);
    let database = temp.path().join("index.sqlite");
    let core = Core::open(&database).unwrap();
    let report = core.scan_folder(&root).unwrap();
    let file = core.snapshot(report.folder.id).unwrap().files[0].clone();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "",
        "fixture/v1",
    )
    .unwrap();
    fs::remove_file(root.join("empty.png")).unwrap();
    core.scan_folder(&root).unwrap();
    drop(core);
    let core = Core::open(database).unwrap();
    assert_eq!(
        core.cached_extraction(&file.image_hash, "fixture/v1")
            .unwrap(),
        Some(String::new())
    );
    assert_eq!(
        core.cached_extraction(&file.image_hash, "fixture/v2")
            .unwrap(),
        None
    );
}

#[test]
fn durable_reprocessing_keeps_existing_text_and_manual_edits_invalidate_old_jobs() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    png(&root.join("note.png"), [1, 2, 3]);
    let database = temp.path().join("index.sqlite");
    let core = Core::open(&database).unwrap();
    let report = core.scan_folder(&root).unwrap();
    let file = core.snapshot(report.folder.id).unwrap().files[0].clone();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "old text",
        "fixture/v1",
    )
    .unwrap();
    core.queue_file(file.id, &file.image_hash, true).unwrap();
    drop(core);
    let core = Arc::new(Core::open(&database).unwrap());
    core.scan_folder(&root).unwrap();
    let job = core.extraction_jobs(file.folder_id).unwrap().remove(0);
    assert!(job.force);
    assert!(job.request_id.is_some());
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("old text")
    );
    core.edit_file(
        file.id,
        &file.image_hash,
        &job.file.record_hash,
        "corrected coffee",
    )
    .unwrap();
    assert!(matches!(
        core.complete_extraction(&job, "stale response", "fixture/v1"),
        Err(Error::ImageChanged)
    ));
    assert!(core.extraction_jobs(file.folder_id).unwrap().is_empty());
    assert_eq!(core.search("corrected", None, 10).unwrap().len(), 1);
    assert_eq!(
        trailer::inspect(&root.join("note.png"))
            .unwrap()
            .trailer
            .unwrap()
            .text,
        "corrected coffee"
    );
    assert!(core
        .edit_file(
            file.id,
            &file.image_hash,
            &job.file.record_hash,
            "stale edit"
        )
        .is_err());
}

#[test]
fn inspector_filename_filter_is_literal_and_pages_large_folders() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    for index in 0..56 {
        png(&root.join(format!("image-{index:02}.png")), [1, 2, 3]);
    }
    let core = Core::open(temp.path().join("index.sqlite")).unwrap();
    let report = core.scan_folder(root).unwrap();
    let page = core.list_files(report.folder.id, "", 0).unwrap();
    assert_eq!((page.total, page.files.len()), (56, 50));
    assert_eq!(
        core.list_files(report.folder.id, "", 50)
            .unwrap()
            .files
            .len(),
        6
    );
    assert_eq!(
        core.list_files(report.folder.id, "IMAGE-01", 0)
            .unwrap()
            .files
            .len(),
        1
    );
    assert!(core
        .list_files(report.folder.id, "%", 0)
        .unwrap()
        .files
        .is_empty());
}

#[test]
fn version_one_database_migration_preserves_text_and_seeds_the_reuse_cache() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    png(&root.join("note.png"), [1, 2, 3]);
    let database = temp.path().join("index.sqlite");
    let core = Core::open(&database).unwrap();
    let report = core.scan_folder(&root).unwrap();
    let file = core.snapshot(report.folder.id).unwrap().files[0].clone();
    core.attach_text(
        file.folder_id,
        &file.relative_path,
        &file.image_hash,
        "migration coffee",
        "fixture/v1",
    )
    .unwrap();
    drop(core);
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("DROP TABLE extraction_failures; DROP TABLE extraction_endpoints; DROP TABLE extraction_jobs; DROP TABLE extraction_cache; DROP INDEX files_image_hash; PRAGMA user_version=1;").unwrap();
    drop(connection);
    let core = Core::open(database).unwrap();
    assert_eq!(
        core.file(file.id).unwrap().text.as_deref(),
        Some("migration coffee")
    );
    assert_eq!(
        core.cached_extraction(&file.image_hash, "fixture/v1")
            .unwrap()
            .as_deref(),
        Some("migration coffee")
    );
    assert_eq!(core.search("migration", None, 10).unwrap().len(), 1);
    core.scan_folder(root).unwrap();
    assert!(core.extraction_jobs(file.folder_id).unwrap().is_empty());
}

#[test]
fn a_failed_database_transaction_keeps_exclusions_active_and_can_recover() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("images");
    fs::create_dir(&root).unwrap();
    png(&root.join("keep.png"), [1, 2, 3]);
    png(&root.join("excluded.png"), [4, 5, 6]);
    let database = temp.path().join("index.sqlite");
    let core = Core::open(&database).unwrap();
    let report = core.scan_folder(&root).unwrap();
    core.set_folder_rules(
        &root,
        &FolderSettings {
            exclusions: vec!["excluded.png".into()],
            ..FolderSettings::default()
        },
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER fixture_failure BEFORE UPDATE ON files BEGIN SELECT RAISE(FAIL, 'fixture failure'); END;").unwrap();
    png(&root.join("keep.png"), [9, 8, 7]);
    assert!(core.reconcile_folder(&root).is_err());
    assert!(core
        .prepare_image(report.folder.id, "excluded.png")
        .is_err());
    connection
        .execute_batch("DROP TRIGGER fixture_failure;")
        .unwrap();
    let recovered = core.reconcile_folder(&root).unwrap();
    assert_eq!((recovered.folder.image_count, recovered.removed), (1, 1));
    assert_eq!(
        recovered.folder.root_hash,
        core.scan_folder(root).unwrap().folder.root_hash
    );
}
