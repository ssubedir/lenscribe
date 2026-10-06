import type {
  DaemonStatus,
  FileDetails,
  FilePage,
  FileRecord,
  Settings,
  ExtractionSettings,
  ModelCatalog,
  MaintenanceStatus,
  MaintenanceReport,
} from "$lib/generated/core";

export interface UpdateInfo {
  version: string;
  notes: string;
}

export interface UpdateStatus {
  revision: number;
  supported: boolean;
  supportMessage: string | null;
  phase: "idle" | "checking" | "available" | "downloading" | "installing" | "restarting" | "error";
  available: UpdateInfo | null;
  progress: number | null;
  lastChecked: number | null;
  error: string | null;
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
  maintenanceStatus(): Promise<MaintenanceStatus>;
  chooseBackupPath(): Promise<string | null>;
  backupDatabase(path: string): Promise<void>;
  rebuildIndex(): Promise<MaintenanceReport>;
  cleanupCache(): Promise<number>;
  updateStatus(): Promise<UpdateStatus>;
  onUpdate(callback: (status: UpdateStatus) => void): Promise<() => void>;
  checkUpdate(): Promise<UpdateStatus>;
  installUpdate(version: string): Promise<void>;
  dispose?(): Promise<void>;
}
