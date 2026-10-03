use tauri_plugin_log::{Builder, RotationStrategy, Target, TargetKind};

fn file_logger(target: TargetKind) -> Builder {
    Builder::new()
        .level(log::LevelFilter::Info)
        // Only application events belong in the persistent log. SDK diagnostics can
        // include request bodies, image bytes, extracted text, and credentials.
        .filter(|metadata| metadata.target().starts_with("lenscribe"))
        .max_file_size(5 * 1024 * 1024)
        // KeepSome names archives to the second, which can leave .bak files
        // behind during bursts. KeepOne bounds disk usage without those archives.
        .rotation_strategy(RotationStrategy::KeepOne)
        .targets([Target::new(target), Target::new(TargetKind::Stderr)])
}

pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    file_logger(TargetKind::LogDir {
        file_name: Some("lenscribe".into()),
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn record(logger: &dyn log::Log, target: &str, message: &str) {
        logger.log(
            &log::Record::builder()
                .target(target)
                .level(log::Level::Info)
                .args(format_args!("{message}"))
                .build(),
        );
        logger.flush();
    }

    #[test]
    fn file_log_appends_timestamped_application_events_and_filters_sdk_payloads() {
        let directory = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_app();
        for _ in 0..2 {
            let (_, _, logger) = file_logger(TargetKind::Folder {
                path: directory.path().into(),
                file_name: Some("lenscribe".into()),
            })
            .split(app.handle())
            .unwrap();
            record(
                logger.as_ref(),
                "lenscribe_core::extraction",
                "Extraction completed",
            );
            record(
                logger.as_ref(),
                "genai",
                "secret-key image-payload private-text",
            );
            record(logger.as_ref(), "reqwest", "secret-key request-url");
        }
        let contents = fs::read_to_string(directory.path().join("lenscribe.log")).unwrap();
        assert_eq!(contents.matches("Extraction completed").count(), 2);
        assert!(contents.contains("INFO"));
        assert!(contents.contains("lenscribe_core::extraction"));
        assert!(contents.lines().all(|line| line.starts_with("[20")));
        assert!(!contents.contains("secret-key"));
        assert!(!contents.contains("image-payload"));
        assert!(!contents.contains("private-text"));
    }

    #[test]
    fn file_log_rotates_and_keeps_disk_usage_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_app();
        let (_, _, logger) = file_logger(TargetKind::Folder {
            path: directory.path().into(),
            file_name: Some("lenscribe".into()),
        })
        .max_file_size(256)
        .split(app.handle())
        .unwrap();
        for index in 0..20 {
            record(
                logger.as_ref(),
                "lenscribe_lib",
                &format!("Entry {index}: {}", "x".repeat(128)),
            );
        }
        drop(logger);
        let files: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].metadata().unwrap().len() <= 256);
        let contents = fs::read_to_string(directory.path().join("lenscribe.log")).unwrap();
        assert!(contents.contains("Entry 19"));
        assert!(!contents.contains("Entry 0:"));
    }
}
