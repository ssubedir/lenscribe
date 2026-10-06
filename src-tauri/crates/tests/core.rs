use std::{
    fs,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use lenscribe_core::{
    http,
    merkle::{self, MerkleTree},
    trailer, Core, Error, WatchEvent,
};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

fn png_bytes(color: [u8; 3]) -> Vec<u8> {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&color).unwrap();
    }
    bytes
}

fn write_png(path: &Path, color: [u8; 3]) -> Vec<u8> {
    let bytes = png_bytes(color);
    fs::write(path, &bytes).unwrap();
    bytes
}

fn setup() -> (TempDir, PathBuf, Arc<Core>) {
    let temporary = tempdir().unwrap();
    let images = temporary.path().join("images");
    fs::create_dir(&images).unwrap();
    let core = Arc::new(Core::open(temporary.path().join("index.wedb")).unwrap());
    (temporary, images, core)
}

#[test]
fn trailer_preserves_decodable_image_and_raw_utf8_text() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("receipt.png");
    let original = write_png(&path, [20, 40, 80]);
    let hash = trailer::hash_bytes(&original);
    let text = "Café receipt\nTotal: £12\nLENSCRIBE-TEXT-V1\nLENSCRIBE-END-V1";
    let image = trailer::write_text(&path, &hash, text, "fixture/model-v1").unwrap();
    assert_eq!(image.image_hash, hash);
    assert_eq!(trailer::original_bytes(&path, &hash).unwrap(), original);
    let appended = fs::read(&path).unwrap();
    assert_eq!(&appended[..original.len()], &original);
    assert!(appended
        .windows(text.len())
        .any(|window| window == text.as_bytes()));
    let mut reader = png::Decoder::new(Cursor::new(appended))
        .read_info()
        .unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut output).unwrap();
    assert_eq!(&output[..3], &[20, 40, 80]);
    let read = trailer::inspect(&path).unwrap().trailer.unwrap();
    assert_eq!(read.text, text);
    assert_eq!(read.processor, "fixture/model-v1");
}

#[test]
fn trailer_replaces_previous_text_and_identical_writes_do_nothing() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let original = write_png(&path, [1, 2, 3]);
    let hash = trailer::hash_bytes(&original);
    trailer::write_text(&path, &hash, &"old text".repeat(100), "model-v1").unwrap();
    trailer::write_text(&path, &hash, "new text", "model-v2").unwrap();
    let before = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    trailer::write_text(&path, &hash, "new text", "model-v2").unwrap();
    assert_eq!(before, fs::read(&path).unwrap());
    assert_eq!(modified, fs::metadata(&path).unwrap().modified().unwrap());
    assert!(!before.windows(8).any(|window| window == b"old text"));
    assert_eq!(
        trailer::inspect(&path).unwrap().trailer.unwrap().text,
        "new text"
    );
    assert_eq!(trailer::original_bytes(&path, &hash).unwrap(), original);
}

#[test]
fn stale_response_cannot_overwrite_a_changed_image() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let original = write_png(&path, [1, 2, 3]);
    let newer = write_png(&path, [9, 8, 7]);
    assert!(matches!(
        trailer::write_text(&path, &trailer::hash_bytes(&original), "stale", "v1"),
        Err(Error::ImageChanged)
    ));
    assert_eq!(fs::read(path).unwrap(), newer);
}

#[test]
fn corrupt_trailer_is_rejected_and_never_overwritten() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let hash = trailer::hash_bytes(&write_png(&path, [1, 2, 3]));
    trailer::write_text(&path, &hash, "unique original text", "v1").unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let offset = bytes
        .windows(20)
        .position(|window| window == b"unique original text")
        .unwrap();
    bytes[offset] = b'X';
    fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        trailer::inspect(&path),
        Err(Error::InvalidTrailer(_))
    ));
    assert!(matches!(
        trailer::write_text(&path, &hash, "replacement", "v1"),
        Err(Error::InvalidTrailer(_))
    ));
    assert_eq!(bytes, fs::read(&path).unwrap());
}

#[test]
fn truncated_footer_cannot_be_mistaken_for_a_new_original_image() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let hash = trailer::hash_bytes(&write_png(&path, [1, 2, 3]));
    trailer::write_text(&path, &hash, "text", "v1").unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes.pop();
    fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        trailer::inspect(&path),
        Err(Error::InvalidTrailer(_))
    ));
    assert!(matches!(
        trailer::write_text(&path, &hash, "replacement", "v1"),
        Err(Error::InvalidTrailer(_))
    ));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn jpeg_original_bytes_and_mime_type_survive_text_attachment() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("pixel.jpg");
    let original = include_bytes!("fixtures/pixel.jpg");
    fs::write(&path, original).unwrap();
    let hash = trailer::hash_bytes(original);
    let image = trailer::write_text(&path, &hash, "JPEG searchable text", "v1").unwrap();
    assert_eq!(image.mime_type, "image/jpeg");
    assert_eq!(trailer::original_bytes(&path, &hash).unwrap(), original);
    assert_eq!(
        trailer::inspect(&path).unwrap().trailer.unwrap().text,
        "JPEG searchable text"
    );
}

#[test]
fn webp_trailers_preserve_pixels_transparency_animation_and_original_bytes() {
    let fixtures: &[(&str, &[u8])] = &[
        ("lossy.webp", include_bytes!("fixtures/pixel-lossy.webp")),
        (
            "lossless.webp",
            include_bytes!("fixtures/pixel-lossless.webp"),
        ),
        ("alpha.webp", include_bytes!("fixtures/pixel-alpha.webp")),
        (
            "animation.webp",
            include_bytes!("fixtures/pixel-animation.webp"),
        ),
    ];
    let decode = |bytes: &[u8]| {
        let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).unwrap();
        let mut frames = Vec::new();
        let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
        if decoder.is_animated() {
            for _ in 0..decoder.num_frames() {
                let duration = decoder.read_frame(&mut pixels).unwrap();
                frames.push((duration, pixels.clone()));
            }
        } else {
            decoder.read_image(&mut pixels).unwrap();
            frames.push((0, pixels));
        }
        (decoder.dimensions(), decoder.has_alpha(), frames)
    };
    let temporary = tempdir().unwrap();
    for (name, original) in fixtures {
        let path = temporary.path().join(name);
        fs::write(&path, original).unwrap();
        let expected_pixels = decode(original);
        let hash = trailer::hash_bytes(original);
        for text in ["WebP searchable café", "Updated WebP text"] {
            let image = trailer::write_text(&path, &hash, text, "fixture/v1").unwrap();
            assert_eq!(image.mime_type, "image/webp");
            assert_eq!(image.image_length, original.len() as u64);
            assert_eq!(image.image_hash, hash);
            assert_eq!(trailer::original_bytes(&path, &hash).unwrap(), *original);
            let appended = fs::read(&path).unwrap();
            assert_eq!(&appended[..original.len()], *original);
            assert!(appended
                .windows(text.len())
                .any(|part| part == text.as_bytes()));
            assert_eq!(decode(&appended), expected_pixels, "{name}");
            assert_eq!(trailer::inspect(&path).unwrap().trailer.unwrap().text, text);
        }
    }
}

#[test]
fn webp_detection_rejects_other_riff_formats_and_incomplete_containers() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("invalid.webp");
    let original = include_bytes!("fixtures/pixel-lossless.webp");
    let mut wave = original.to_vec();
    wave[8..12].copy_from_slice(b"WAVE");
    let mut oversized = original.to_vec();
    oversized[4..8].copy_from_slice(&(original.len() as u32 + 100).to_le_bytes());
    let mut odd = original.to_vec();
    odd[4..8].copy_from_slice(&13u32.to_le_bytes());
    let mut empty = original[..12].to_vec();
    empty[4..8].copy_from_slice(&4u32.to_le_bytes());
    for invalid in [
        wave,
        oversized,
        odd,
        empty,
        original[..11].to_vec(),
        original[..20].to_vec(),
    ] {
        fs::write(&path, &invalid).unwrap();
        assert!(matches!(
            trailer::inspect(&path),
            Err(Error::UnsupportedImage(_))
        ));
        assert!(matches!(
            trailer::write_text(&path, &trailer::hash_bytes(&invalid), "text", "v1"),
            Err(Error::UnsupportedImage(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), invalid);
    }
}

#[test]
fn webp_files_scan_prepare_search_and_reopen_with_stable_merkle_identity() {
    let (temporary, images, core) = setup();
    fs::create_dir(images.join("nested")).unwrap();
    let original = include_bytes!("fixtures/pixel-alpha.webp");
    let relative = "nested/receipt.WEBP";
    let path = images.join(relative);
    fs::write(&path, original).unwrap();
    let initial = core.scan_folder(&images).unwrap();
    assert_eq!(initial.folder.image_count, 1);
    assert!(initial.issues.is_empty());
    let prepared = core.prepare_image(initial.folder.id, relative).unwrap();
    assert_eq!(prepared.mime_type, "image/webp");
    assert_eq!(prepared.bytes, original);
    core.attach_text(
        initial.folder.id,
        relative,
        &prepared.image_hash,
        "WebP coffee receipt",
        "fixture/v1",
    )
    .unwrap();
    let hits = core.search("coffee", None, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file.relative_path, relative);
    let stable = core.scan_folder(&images).unwrap();
    assert_eq!(stable.changed, 0);
    assert_ne!(initial.folder.root_hash, stable.folder.root_hash);
    assert_eq!(
        core.prepare_image(initial.folder.id, relative)
            .unwrap()
            .bytes,
        original
    );
    drop(core);
    let reopened = Core::open(temporary.path().join("index.wedb")).unwrap();
    assert_eq!(
        reopened
            .snapshot(initial.folder.id)
            .unwrap()
            .folder
            .root_hash,
        stable.folder.root_hash
    );
    assert_eq!(reopened.search("coffee", None, 10).unwrap().len(), 1);
}

#[test]
fn forged_trailer_lengths_are_rejected_before_allocation() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let hash = trailer::hash_bytes(&write_png(&path, [1, 2, 3]));
    trailer::write_text(&path, &hash, "text", "v1").unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let marker = b"\nLENSCRIBE-END-V1 ";
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    bytes[offset..offset + 20].copy_from_slice(b"18446744073709551615");
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        trailer::inspect(&path),
        Err(Error::InvalidTrailer(_))
    ));
}

#[test]
fn original_foreign_trailing_bytes_are_preserved() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let mut original = write_png(&path, [1, 2, 3]);
    original.extend_from_slice(b"existing unmarked trailer");
    fs::write(&path, &original).unwrap();
    let hash = trailer::hash_bytes(&original);
    trailer::write_text(&path, &hash, "new searchable text", "v1").unwrap();
    assert_eq!(trailer::original_bytes(&path, &hash).unwrap(), original);
}

#[test]
fn oversized_and_unsupported_inputs_do_not_change_files() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("not-an-image.png");
    fs::write(&path, b"not an image").unwrap();
    assert!(matches!(
        trailer::inspect(&path),
        Err(Error::UnsupportedImage(_))
    ));
    let original = write_png(&path, [1, 2, 3]);
    let hash = trailer::hash_bytes(&original);
    assert!(matches!(
        trailer::write_text(&path, &hash, "text", ""),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        trailer::write_text(&path, &hash, &"x".repeat(trailer::MAX_TEXT_BYTES + 1), "v1"),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn merkle_roots_are_order_independent_and_incremental_updates_match_rebuilds() {
    let first = merkle::record_hash(&trailer::hash_bytes(b"first image"), None).unwrap();
    let second = merkle::record_hash(&trailer::hash_bytes(b"second image"), None).unwrap();
    let mut a = MerkleTree::new();
    a.insert("photos/one.png", &first).unwrap();
    a.insert("screenshots/nested/two.png", &second).unwrap();
    let mut b = MerkleTree::new();
    b.insert("screenshots/nested/two.png", &second).unwrap();
    b.insert("photos/one.png", &first).unwrap();
    assert_eq!(a.root_hash(), b.root_hash());
    let old_root = a.root_hash();
    a.insert("photos/one.png", &second).unwrap();
    b.insert("photos/one.png", &second).unwrap();
    assert_ne!(old_root, a.root_hash());
    assert_eq!(a.root_hash(), b.root_hash());
    a.remove("screenshots/nested/two.png").unwrap();
    let mut rebuilt = MerkleTree::new();
    rebuilt.insert("photos/one.png", &second).unwrap();
    assert_eq!(a.root_hash(), rebuilt.root_hash());
    a.remove("photos/one.png").unwrap();
    assert_eq!(a.root_hash(), MerkleTree::new().root_hash());
}

#[test]
fn merkle_records_commit_text_processing_version_and_names() {
    let image = trailer::hash_bytes(b"image");
    let pending = merkle::record_hash(&image, None).unwrap();
    let empty = merkle::record_hash(&image, Some(("", "model-v1"))).unwrap();
    let processed = merkle::record_hash(&image, Some(("coffee", "model-v1"))).unwrap();
    let upgraded = merkle::record_hash(&image, Some(("coffee", "model-v2"))).unwrap();
    assert_ne!(pending, empty);
    assert_ne!(empty, processed);
    assert_ne!(processed, upgraded);
    let mut tree = MerkleTree::new();
    tree.insert("one.png", &processed).unwrap();
    let before = tree.root_hash();
    tree.remove("one.png").unwrap();
    tree.insert("renamed.png", &processed).unwrap();
    assert_ne!(before, tree.root_hash());
}

#[test]
fn index_text_search_and_database_reopen_preserve_identity() {
    let (temporary, images, core) = setup();
    let original = write_png(&images.join("receipt.png"), [1, 2, 3]);
    let report = core.scan_folder(&images).unwrap();
    assert_eq!(report.changed, 1);
    let folder_id = report.folder.id;
    let pending = core.snapshot(folder_id).unwrap().files.remove(0);
    assert!(core.file(pending.id).unwrap().text.is_none());
    let prepared = core.prepare_image(folder_id, "receipt.png").unwrap();
    assert_eq!(prepared.bytes, original);
    assert_eq!(prepared.mime_type, "image/png");
    let processed = core
        .attach_text(
            folder_id,
            "receipt.png",
            &prepared.image_hash,
            "Coffee receipt €12",
            "fixture/v1",
        )
        .unwrap();
    assert_eq!(pending.id, processed.file.id);
    assert_eq!(pending.image_hash, processed.file.image_hash);
    assert_ne!(pending.record_hash, processed.file.record_hash);
    let hits = core.search("coffee", None, 20).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file.id, pending.id);
    assert!(hits[0].snippet.contains("[Coffee]"));
    let stable = core.scan_folder(&images).unwrap();
    assert_eq!(stable.changed, 0);
    assert_eq!(stable.removed, 0);
    assert_ne!(report.folder.root_hash, stable.folder.root_hash);
    let root = stable.folder.root_hash;
    drop(core);
    let reopened = Core::open(temporary.path().join("index.wedb")).unwrap();
    assert_eq!(reopened.snapshot(folder_id).unwrap().folder.root_hash, root);
    assert_eq!(
        reopened.search("coffee", None, 20).unwrap()[0].file.id,
        pending.id
    );
    assert_eq!(
        reopened
            .prepare_image(folder_id, "receipt.png")
            .unwrap()
            .bytes,
        original
    );
}

#[test]
fn imported_trailers_and_search_updates_do_not_leave_stale_terms() {
    let (_temporary, images, core) = setup();
    let path = images.join("receipt.png");
    let hash = trailer::hash_bytes(&write_png(&path, [1, 2, 3]));
    trailer::write_text(&path, &hash, "coffee", "v1").unwrap();
    let folder_id = core.scan_folder(&images).unwrap().folder.id;
    assert_eq!(core.search("coffee", None, 20).unwrap().len(), 1);
    core.attach_text(folder_id, "receipt.png", &hash, "tea", "v1")
        .unwrap();
    assert!(core.search("coffee", None, 20).unwrap().is_empty());
    assert_eq!(core.search("tea", None, 20).unwrap().len(), 1);
    fs::remove_file(path).unwrap();
    let report = core.scan_folder(images).unwrap();
    assert_eq!(report.removed, 1);
    assert_eq!(report.folder.root_hash, MerkleTree::new().root_hash());
    assert!(core.search("tea", None, 20).unwrap().is_empty());
}

#[test]
fn invalid_images_are_reported_and_do_not_block_valid_images() {
    let (_temporary, images, core) = setup();
    write_png(&images.join("valid.PNG"), [1, 2, 3]);
    fs::write(images.join("broken.png"), b"unfinished image").unwrap();
    fs::write(images.join("ignore.txt"), b"not an image").unwrap();
    let report = core.scan_folder(images).unwrap();
    assert_eq!(report.folder.image_count, 1);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].path, "broken.png");
}

#[test]
fn identical_content_in_different_roots_has_identical_merkle_roots() {
    let (temporary, images, core) = setup();
    let other = temporary.path().join("other");
    fs::create_dir(&other).unwrap();
    let original = write_png(&images.join("same.png"), [1, 2, 3]);
    fs::write(other.join("same.png"), original).unwrap();
    let a = core.scan_folder(&images).unwrap();
    let b = core.scan_folder(&other).unwrap();
    assert_eq!(a.folder.root_hash, b.folder.root_hash);
    core.attach_text(
        a.folder.id,
        "same.png",
        &core.snapshot(a.folder.id).unwrap().files[0].image_hash,
        "coffee",
        "v1",
    )
    .unwrap();
    assert_eq!(
        core.search("coffee", Some(a.folder.id), 20).unwrap().len(),
        1
    );
    assert!(core
        .search("coffee", Some(b.folder.id), 20)
        .unwrap()
        .is_empty());
    assert!(core.search("   ", None, 20).unwrap().is_empty());
    assert!(core.search("coffee OR tea", None, 20).unwrap().is_empty());
    assert_eq!(core.search("\"coffee\"", None, 20).unwrap().len(), 1);
}

#[test]
fn relative_paths_cannot_escape_the_selected_folder() {
    let (_temporary, images, core) = setup();
    let folder_id = core.scan_folder(&images).unwrap().folder.id;
    for path in [
        "../outside.png",
        "/absolute.png",
        "C:/outside.png",
        "nested\\outside.png",
        "a//b.png",
        "./image.png",
    ] {
        assert!(
            matches!(
                core.prepare_image(folder_id, path),
                Err(Error::InvalidInput(_))
            ),
            "{path}"
        );
        assert!(
            MerkleTree::new()
                .insert(path, &trailer::hash_bytes(b"image"))
                .is_err(),
            "{path}"
        );
    }
}

#[test]
fn watcher_indexes_creates_renames_and_removals_and_stops_cleanly() {
    let (_temporary, images, core) = setup();
    let (sender, receiver) = mpsc::channel();
    let initial = core
        .watch_folder(
            &images,
            Arc::new(move |event| {
                sender.send(event).unwrap();
            }),
        )
        .unwrap();
    write_png(&images.join("new.png"), [1, 2, 3]);
    let WatchEvent::Updated(created) = receiver.recv_timeout(Duration::from_secs(10)).unwrap()
    else {
        panic!("watch failed")
    };
    assert_eq!(created.folder.image_count, 1);
    fs::rename(images.join("new.png"), images.join("renamed.png")).unwrap();
    let WatchEvent::Updated(renamed) = receiver.recv_timeout(Duration::from_secs(10)).unwrap()
    else {
        panic!("watch failed")
    };
    assert_eq!(renamed.changed, 1);
    assert_eq!(renamed.removed, 1);
    fs::remove_file(images.join("renamed.png")).unwrap();
    let WatchEvent::Updated(removed) = receiver.recv_timeout(Duration::from_secs(10)).unwrap()
    else {
        panic!("watch failed")
    };
    assert_eq!(removed.folder.image_count, 0);
    core.unwatch_folder(initial.folder.id).unwrap();
    assert!(core.watch_status().unwrap().is_empty());
}

#[tokio::test]
async fn http_serves_search_text_and_meaningful_missing_text_errors() {
    let (_temporary, images, core) = setup();
    write_png(&images.join("receipt.png"), [1, 2, 3]);
    let folder_id = core.scan_folder(&images).unwrap().folder.id;
    let file = core.snapshot(folder_id).unwrap().files.remove(0);
    let app = http::router(core.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/files/{}/text", file.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    core.attach_text(
        folder_id,
        "receipt.png",
        &file.image_hash,
        "Café coffee receipt",
        "v1",
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/search?q=coffee&folderId={folder_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let hits: Vec<lenscribe_core::SearchHit> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file.id, file.id);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/search/page?q=cofee&folderId={folder_id}&limit=1"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let page: lenscribe_core::SearchPage = serde_json::from_slice(&bytes).unwrap();
    let inspector = core.find_files(folder_id, "cofee", 0, true).unwrap();
    assert_eq!(page.total, inspector.total);
    assert_eq!(page.hits[0].file.id, inspector.files[0].id);
    assert!(page.fuzzy_applied);
    assert!(page.hits[0].snippet.contains("[coffee]"));
    for (query, expected) in [
        ("q=cofee&fuzzy=false", 0),
        ("q=cofee&offset=1&limit=1", 0),
        ("q=coffee%20OR%20unrelated", 0),
        ("q=cofee", 1),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/search?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let results: Vec<lenscribe_core::SearchHit> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(results.len(), expected, "query: {query}");
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/search/page?q=coffee&folderId=999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/files/{}/text", file.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        "Café coffee receipt".as_bytes()
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/files/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/search?q=coffee")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[test]
fn changed_original_bytes_in_a_processed_image_invalidate_the_trailer() {
    let temporary = tempdir().unwrap();
    let path = temporary.path().join("note.png");
    let hash = trailer::hash_bytes(&write_png(&path, [1, 2, 3]));
    trailer::write_text(&path, &hash, "text", "v1").unwrap();
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    file.seek(SeekFrom::Start(20)).unwrap();
    let mut byte = [0];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 1;
    file.seek(SeekFrom::Start(20)).unwrap();
    file.write_all(&byte).unwrap();
    drop(file);
    assert!(matches!(
        trailer::inspect(&path),
        Err(Error::InvalidTrailer(_))
    ));
}
