//! The serialized Rust DTOs are the source of truth for the frontend contract.
use ts_rs::{Config, TS};

use crate::{
    daemon::{DaemonFolderStatus, DaemonIssue, DaemonStatus},
    extraction::{ExtractionIssue, ExtractionPhase, ExtractionStatus},
    llm::{ModelCatalog, VisionModel},
    settings::{
        ApiSettings, ExtractionSettings, FolderSettings, LlmProvider, SavedConnection, Settings,
        Theme,
    },
    FileDetails, FilePage, FileRecord, FolderRecord, FolderSnapshot, ScanIssue, ScanReport,
    SearchHit, WatchEvent, WatchFailure, WatchStatus,
};

pub fn typescript() -> String {
    // Tauri's JSON transport sends integers as numbers, not JavaScript bigint values.
    let config = Config::new().with_large_int("number");
    let declarations = [
        FolderRecord::decl(&config),
        FileRecord::decl(&config),
        FileDetails::decl(&config),
        FilePage::decl(&config),
        FolderSnapshot::decl(&config),
        ScanIssue::decl(&config),
        ScanReport::decl(&config),
        SearchHit::decl(&config),
        WatchStatus::decl(&config),
        WatchFailure::decl(&config),
        WatchEvent::decl(&config),
        Theme::decl(&config),
        FolderSettings::decl(&config),
        ApiSettings::decl(&config),
        LlmProvider::decl(&config),
        ExtractionSettings::decl(&config),
        SavedConnection::decl(&config),
        VisionModel::decl(&config),
        ModelCatalog::decl(&config),
        Settings::decl(&config),
        ExtractionPhase::decl(&config),
        ExtractionIssue::decl(&config),
        ExtractionStatus::decl(&config),
        DaemonIssue::decl(&config),
        DaemonFolderStatus::decl(&config),
        DaemonStatus::decl(&config),
    ];
    let mut output =
        String::from("// Generated from Rust. Run bun run types:generate; do not edit.\n\n");
    for declaration in declarations {
        output.push_str("export ");
        for line in declaration.lines() {
            output.push_str(line.trim_end());
            output.push('\n');
        }
        output.push('\n');
    }
    output
}
