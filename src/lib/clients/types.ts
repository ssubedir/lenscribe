import type {
  DaemonStatus,
  FileDetails,
  FilePage,
  FileRecord,
  Settings,
  ExtractionSettings,
  ModelCatalog,
} from "$lib/generated/core";

export interface UpdateInfo {
  version: string;
  notes: string;
}

export interface AppClient {
  readonly mode: "desktop" | "preview";
  readonly pollInterval: number;
  status(): Promise<DaemonStatus>;
  save(settings: Settings): Promise<DaemonStatus>;
  retry(): Promise<DaemonStatus>;
  chooseFolder(): Promise<string | null>;
  onError(callback: (message: string) => void): Promise<() => void>;
  listFiles(folderId: number, query: string, offset: number, fuzzy?: boolean): Promise<FilePage>;
  fileDetails(fileId: number): Promise<FileDetails>;
  filePreview(fileId: number): Promise<string>;
  editFile(file: FileRecord, text: string): Promise<{ file: FileDetails; message: string }>;
  queueFile(file: FileRecord, force: boolean, available: boolean): Promise<string>;
  discoverModels(settings: ExtractionSettings): Promise<ModelCatalog>;
  checkUpdate(): Promise<UpdateInfo | null>;
  installUpdate(onProgress: (percent: number | null) => void): Promise<void>;
  dispose?(): Promise<void>;
}
