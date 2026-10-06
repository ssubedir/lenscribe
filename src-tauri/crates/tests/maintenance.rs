use lenscribe_core::{trailer, Core};
use std::fs;
use tempfile::tempdir;

#[test]
fn live_backup_preserves_cache_queue_and_text_and_never_overwrites_files() {
    let temp = tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    fs::write(
        images.join("receipt.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let core = Core::open(temp.path().join("index.wedb")).unwrap();
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let file = core.snapshot(folder).unwrap().files.remove(0);
    core.attach_text(
        folder,
        &file.relative_path,
        &file.image_hash,
        "coffee receipt",
        "fixture/v1",
    )
    .unwrap();
    core.queue_file(file.id, &file.image_hash, true).unwrap();
    let backup = temp.path().join("backup.lenscribe-backup");
    core.backup_database(&backup).unwrap();
    let restored = Core::restore_database(&backup, temp.path().join("restored.wedb")).unwrap();
    assert_eq!(
        restored.file(file.id).unwrap().text.as_deref(),
        Some("coffee receipt")
    );
    assert_eq!(
        restored
            .cached_extraction(&file.image_hash, "fixture/v1")
            .unwrap()
            .as_deref(),
        Some("coffee receipt")
    );
    assert!(restored.extraction_jobs(folder).unwrap()[0].force);
    assert_eq!(restored.search("cofee", Some(folder), 20).unwrap().len(), 1);
    assert_eq!(restored.maintenance_status().unwrap().indexed_files, 1);
    drop(restored);
    let original = fs::read(&backup).unwrap();
    assert!(core.backup_database(&backup).is_err());
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert!(core
        .backup_database(temp.path().join("index.wedb"))
        .is_err());
}

#[test]
fn rebuild_reimports_trailers_repairs_search_and_retains_unavailable_folders() {
    let temp = tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    let image = images.join("receipt.jpg");
    fs::write(&image, include_bytes!("fixtures/pixel.jpg")).unwrap();
    let index = temp.path().join("index.wedb");
    let core = Core::open(&index).unwrap();
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let file = core.snapshot(folder).unwrap().files.remove(0);
    core.attach_text(
        folder,
        &file.relative_path,
        &file.image_hash,
        "coffee receipt",
        "fixture/v1",
    )
    .unwrap();

    trailer::write_text(&image, &file.image_hash, "espresso receipt", "fixture/v1").unwrap();
    let before = fs::read(&image).unwrap();
    let report = core.rebuild_index().unwrap();
    assert_eq!(
        (
            report.scanned_folders,
            report.changed_files,
            report.issues.len()
        ),
        (1, 1, 0)
    );
    assert_eq!(core.search("espreso", None, 20).unwrap().len(), 1);
    assert!(core.search("cofee", None, 20).unwrap().is_empty());
    assert_eq!(fs::read(&image).unwrap(), before);
    fs::rename(&images, temp.path().join("offline")).unwrap();
    let report = core.rebuild_index().unwrap();
    assert_eq!((report.scanned_folders, report.issues.len()), (0, 1));
    assert_eq!(core.snapshot(folder).unwrap().files.len(), 1);
}

#[test]
fn restore_rejects_damaged_inconsistent_or_future_backups_before_creating_storage() {
    let temp = tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    fs::write(
        images.join("receipt.jpg"),
        include_bytes!("fixtures/pixel.jpg"),
    )
    .unwrap();
    let core = Core::open(temp.path().join("index.wedb")).unwrap();
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let file = core.snapshot(folder).unwrap().files.remove(0);
    core.attach_text(
        folder,
        &file.relative_path,
        &file.image_hash,
        "coffee receipt",
        "fixture/v1",
    )
    .unwrap();
    let backup = temp.path().join("original.lenscribe-backup");
    core.backup_database(&backup).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
    for kind in [
        "checksum",
        "future",
        "duplicate",
        "missing-body",
        "body-hash",
        "missing-folder",
    ] {
        let mut altered = original.clone();
        match kind {
            "checksum" => altered["checksum"] = "wrong".into(),
            "future" => altered["version"] = 99.into(),
            "duplicate" => {
                let records = altered["records"].as_array_mut().unwrap();
                records.push(records[0].clone());
            }
            "missing-body" => altered["records"]
                .as_array_mut()
                .unwrap()
                .retain(|row| !row[0].as_str().unwrap().starts_with("texts/")),
            "body-hash" => {
                let row = altered["records"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|row| row[0].as_str().unwrap().starts_with("texts/"))
                    .unwrap();
                row[1] = "tampered text".into();
            }
            "missing-folder" => altered["records"]
                .as_array_mut()
                .unwrap()
                .retain(|row| !row[0].as_str().unwrap().starts_with("folders/")),
            _ => unreachable!(),
        }
        // Reference checks still apply even when the outer checksum is valid.
        if kind != "checksum" {
            altered["checksum"] =
                trailer::hash_bytes(&serde_json::to_vec(&altered["records"]).unwrap()).into();
        }
        let damaged = temp.path().join(format!("{kind}.lenscribe-backup"));
        fs::write(&damaged, serde_json::to_vec(&altered).unwrap()).unwrap();
        let destination = temp.path().join(format!("{kind}.wedb"));
        assert!(
            Core::restore_database(&damaged, &destination).is_err(),
            "{kind}"
        );
        assert!(
            !destination.exists(),
            "{kind} created storage before validation"
        );
    }
    let existing = temp.path().join("existing.wedb");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), b"untouched").unwrap();
    assert!(Core::restore_database(&backup, &existing).is_err());
    assert_eq!(fs::read(existing.join("keep")).unwrap(), b"untouched");
}

#[test]
fn storage_has_one_owner_and_reopens_after_it_is_dropped() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("index.wedb");
    let core = Core::open(&path).unwrap();
    assert!(Core::open(&path).is_err());
    drop(core);
    assert!(Core::open(&path).is_ok());
}

#[test]
fn cache_cleanup_retains_referenced_results_and_does_not_touch_images() {
    let temp = tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    let image = images.join("receipt.jpg");
    fs::write(&image, include_bytes!("fixtures/pixel.jpg")).unwrap();
    let core = Core::open(temp.path().join("index.wedb")).unwrap();
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let file = core.snapshot(folder).unwrap().files.remove(0);
    core.attach_text(
        folder,
        &file.relative_path,
        &file.image_hash,
        "coffee",
        "fixture/v1",
    )
    .unwrap();
    core.attach_text(
        folder,
        &file.relative_path,
        &file.image_hash,
        "croissant",
        "fixture/v2",
    )
    .unwrap();
    let before = fs::read(&image).unwrap();
    let status = core.maintenance_status().unwrap();
    assert_eq!(
        (
            status.indexed_files,
            status.cached_extractions,
            status.unused_cached_extractions,
            status.cache_bytes
        ),
        (1, 2, 1, 15)
    );
    assert_eq!(core.cleanup_cache().unwrap(), 1);
    assert!(core
        .cached_extraction(&file.image_hash, "fixture/v1")
        .unwrap()
        .is_none());
    assert_eq!(
        core.cached_extraction(&file.image_hash, "fixture/v2")
            .unwrap()
            .as_deref(),
        Some("croissant")
    );
    assert_eq!(core.cleanup_cache().unwrap(), 0);
    assert_eq!(fs::read(&image).unwrap(), before);
    assert_eq!(core.search("croissant", None, 20).unwrap().len(), 1);
}

#[cfg(windows)]
#[test]
fn temporarily_locked_images_keep_their_existing_index_entries() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempdir().unwrap();
    let images = temp.path().join("images");
    fs::create_dir(&images).unwrap();
    let image = images.join("receipt.jpg");
    fs::write(&image, include_bytes!("fixtures/pixel.jpg")).unwrap();
    let core = Core::open(temp.path().join("index.wedb")).unwrap();
    let folder = core.scan_folder(&images).unwrap().folder.id;
    let file = core.snapshot(folder).unwrap().files.remove(0);
    core.attach_text(
        folder,
        &file.relative_path,
        &file.image_hash,
        "coffee",
        "fixture/v1",
    )
    .unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&image)
        .unwrap();
    let report = core.scan_folder(&images).unwrap();
    assert_eq!(report.removed, 0);
    assert_eq!(core.search("coffee", Some(folder), 20).unwrap().len(), 1);
    drop(locked);
    assert!(core.scan_folder(&images).unwrap().issues.is_empty());
}
