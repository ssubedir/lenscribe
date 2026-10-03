import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  DaemonStatus,
  FileDetails,
  FilePage,
  FileRecord,
  FolderRecord,
  FolderSnapshot,
  ScanReport,
  SearchHit,
  Settings,
  WatchEvent,
  WatchStatus,
  ExtractionSettings,
  ModelCatalog,
} from "$lib/generated/core";
export type * from "$lib/generated/core";

export const daemonStatus = () => invoke<DaemonStatus>("daemon_status");
export const saveSettings = (settings: Settings) =>
  invoke<DaemonStatus>("save_settings", { settings });
export const retryDaemon = () => invoke<DaemonStatus>("retry_daemon");
export const discoverModels = (settings: ExtractionSettings) =>
  invoke<ModelCatalog>("discover_llm_models", { settings });
export const prepareUpdateInstall = () => invoke<void>("prepare_update_install");

export const scanFolder = (path: string) => invoke<ScanReport>("scan_folder", { path });
export const listFolders = () => invoke<FolderRecord[]>("list_folders");
export const folderSnapshot = (folderId: number) =>
  invoke<FolderSnapshot>("folder_snapshot", { folderId });
export const fileDetails = (fileId: number) => invoke<FileDetails>("file_details", { fileId });
export const listFiles = (folderId: number, query = "", offset = 0) =>
  invoke<FilePage>("list_files", { folderId, query, offset });
export const filePreview = (fileId: number) => invoke<string>("file_preview", { fileId });
export const queueFile = (file: FileRecord, force: boolean) =>
  invoke<DaemonStatus>("queue_file", { fileId: file.id, expectedImageHash: file.imageHash, force });
export const editFile = (file: FileRecord, text: string) =>
  invoke<FileDetails>("edit_file", {
    fileId: file.id,
    expectedImageHash: file.imageHash,
    expectedRecordHash: file.recordHash,
    text,
  });
export const watchFolder = (path: string) => invoke<ScanReport>("watch_folder", { path });
export const unwatchFolder = (folderId: number) => invoke<void>("unwatch_folder", { folderId });
export const watchStatus = () => invoke<WatchStatus[]>("watch_status");
export const startApi = (port = 47831) => invoke<string>("start_api", { port });
export const stopApi = () => invoke<void>("stop_api");

export function attachText(
  file: FileRecord,
  text: string,
  processor: string,
): Promise<FileDetails> {
  return invoke("attach_text", {
    folderId: file.folderId,
    relativePath: file.relativePath,
    expectedImageHash: file.imageHash,
    text,
    processor,
  });
}

export function searchFiles(query: string, folderId?: number, limit = 20): Promise<SearchHit[]> {
  return invoke("search_files", { query, folderId: folderId ?? null, limit });
}

export function onWatchEvent(callback: (event: WatchEvent) => void): Promise<UnlistenFn> {
  return listen<WatchEvent>("lenscribe://watch", (event) => callback(event.payload));
}
