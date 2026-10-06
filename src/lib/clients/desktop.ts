import { open, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import * as core from "$lib/core";
import type { AppClient } from "./types";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { createUpdater } from "./updater";

export function createDesktopClient(): AppClient {
  return {
    ...createUpdater({
      check: () => check({ timeout: 20000 }),
      prepare: core.prepareUpdateInstall,
      relaunch,
    }),
    mode: "desktop",
    pollInterval: 2000,
    status: core.daemonStatus,
    save: core.saveSettings,
    retry: core.retryDaemon,
    async chooseFolder() {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose a folder to monitor",
      });
      return typeof selected === "string" ? selected : null;
    },
    onError: (callback) =>
      listen<string>("lenscribe://daemon-error", (event) => callback(event.payload)),
    listFiles: core.listFiles,
    fileDetails: core.fileDetails,
    filePreview: core.filePreview,
    discoverModels: core.discoverModels,
    maintenanceStatus: core.maintenanceStatus,
    backupDatabase: core.backupDatabase,
    rebuildIndex: core.rebuildIndex,
    cleanupCache: core.cleanupCache,
    chooseBackupPath: () =>
      saveDialog({
        title: "Export Lenscribe database backup",
        defaultPath: `lenscribe-backup-${Date.now()}.lenscribe-backup`,
        filters: [{ name: "Lenscribe backup", extensions: ["lenscribe-backup"] }],
      }),
    async editFile(file, text) {
      return {
        file: await core.editFile(file, text),
        message: "Text saved to the image and search index.",
      };
    },
    async queueFile(file, force, available) {
      await core.queueFile(file, force);
      return !available
        ? "Queued. Enable extraction and resume monitoring to process it."
        : force
          ? "Queued for fresh extraction. Existing text stays until the new result succeeds."
          : "Queued for another attempt.";
    },
  };
}
