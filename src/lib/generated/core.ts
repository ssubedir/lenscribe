// Generated from Rust. Run bun run types:generate; do not edit.

export type FolderRecord = { id: number, path: string, rootHash: string, imageCount: number, };

export type FileRecord = { id: number, folderId: number, relativePath: string, imageHash: string, imageLength: number, textHash: string | null, recordHash: string, processor: string | null, };

export type FileDetails = { text: string | null, id: number, folderId: number, relativePath: string, imageHash: string, imageLength: number, textHash: string | null, recordHash: string, processor: string | null, };

export type FilePage = { files: Array<FileRecord>, total: number, };

export type FolderSnapshot = { folder: FolderRecord, files: Array<FileRecord>, };

export type ScanIssue = { path: string, error: string, };

export type ScanReport = { folder: FolderRecord, changed: number, removed: number, inspected: number, issues: Array<ScanIssue>, };

export type SearchHit = { folderPath: string, snippet: string, id: number, folderId: number, relativePath: string, imageHash: string, imageLength: number, textHash: string | null, recordHash: string, processor: string | null, };

export type WatchStatus = { folderId: number, path: string, lastError: string | null, };

export type WatchFailure = { folderId: number, error: string, };

export type WatchEvent = { "type": "updated", "data": ScanReport } | { "type": "failed", "data": WatchFailure };

export type Theme = "light" | "dark";

export type FolderSettings = { path: string, enabled: boolean, exclusions: Array<string>,
/**
 * Zero leaves indexing unrestricted. The extraction client's size limit still applies.
 */
maxImageMib: number, };

export type ApiSettings = { enabled: boolean, port: number, };

export type LlmProvider = "custom" | "openai" | "anthropic" | "gemini" | "openRouter" | "ollama" | "groq" | "xai";

export type ExtractionSettings = { enabled: boolean, provider: LlmProvider, baseUrl: string, model: string,
/**
 * Saved with the other settings. Empty means no authentication.
 */
apiKey: string, prompt: string, maxTokens: number, timeoutSeconds: number, concurrency: number,
/**
 * Zero disables the per-minute request limit.
 */
requestsPerMinute: number, };

export type SavedConnection = { provider: LlmProvider, baseUrl: string, model: string, apiKey: string, };

export type VisionModel = { id: string, name: string, vision: boolean | null, };

export type ModelCatalog = { models: Array<VisionModel>, truncated: boolean, };

export type Settings = { version: number, folders: Array<FolderSettings>, monitoringPaused: boolean, startMinimized: boolean, startAtLogin: boolean, theme: Theme, api: ApiSettings, extraction: ExtractionSettings, connections: Array<SavedConnection>, };

export type ExtractionPhase = "disabled" | "paused" | "idle" | "extracting" | "needsConfiguration";

export type ExtractionIssue = { folderId: number, relativePath: string, error: string, retrying: boolean, attempts: number, retryAtMs: number | null, };

export type ExtractionStatus = { phase: ExtractionPhase, currentFile: string | null, activeFiles: Array<string>, completedImages: number, failedImages: number, lastError: string | null, issues: Array<ExtractionIssue>, };

export type DaemonIssue = { source: string, error: string, };

export type DaemonFolderStatus = { path: string, enabled: boolean, folderId: number | null, watching: boolean, imageCount: number, pendingImages: number, rootHash: string | null, lastError: string | null, };

export type DaemonStatus = { settings: Settings, watchers: Array<WatchStatus>, pendingImages: number, totalImages: number, processedImages: number, folderStatuses: Array<DaemonFolderStatus>, apiUrl: string | null, issues: Array<DaemonIssue>, extraction: ExtractionStatus, };

