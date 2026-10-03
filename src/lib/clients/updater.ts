import type { DownloadEvent } from "@tauri-apps/plugin-updater";
import type { AppClient } from "./types";

interface UpdatePackage {
  version: string;
  body?: string;
  download(onEvent: (event: DownloadEvent) => void, options: { timeout: number }): Promise<void>;
  install(): Promise<void>;
  close(): Promise<void>;
}

// The official plugin verifies the signature during download, before monitoring stops.
export function createUpdater(dependencies: {
  check(): Promise<UpdatePackage | null>;
  prepare(): Promise<void>;
  relaunch(): Promise<void>;
}): Pick<AppClient, "checkUpdate" | "installUpdate" | "dispose"> {
  let update: UpdatePackage | null = null;
  let busy = false;
  let disposed = false;
  return {
    async checkUpdate() {
      if (busy || disposed) throw new Error("An update operation is already in progress.");
      busy = true;
      try {
        await update?.close();
        update = null;
        const result = await dependencies.check();
        if (disposed) {
          await result?.close();
          return null;
        }
        update = result;
        return update ? { version: update.version, notes: update.body ?? "" } : null;
      } catch {
        throw new Error(
          "Could not check for updates. Check your internet connection and that a signed release is available on GitHub.",
        );
      } finally {
        busy = false;
      }
    },
    async installUpdate(onProgress) {
      if (!update || busy || disposed) throw new Error("Check for updates before installing.");
      busy = true;
      const selected = update;
      let stopped = false;
      try {
        let total = 0;
        let downloaded = 0;
        await selected.download(
          (event) => {
            if (event.event === "Started") total = event.data.contentLength ?? 0;
            if (event.event === "Progress") downloaded += event.data.chunkLength;
            onProgress(
              event.event === "Finished"
                ? 100
                : total
                  ? Math.min(100, Math.round((downloaded / total) * 100))
                  : null,
            );
          },
          { timeout: 300000 },
        );
        stopped = true;
        await dependencies.prepare();
        await selected.install();
        await dependencies.relaunch();
      } catch {
        throw new Error(
          stopped
            ? "Update installation failed. Restart Lenscribe to resume monitoring, then try again."
            : "Could not download or verify the update. Monitoring is still running; try again later.",
        );
      } finally {
        update = null;
        await selected.close().catch(() => {});
        busy = false;
      }
    },
    async dispose() {
      disposed = true;
      if (!busy) {
        const selected = update;
        update = null;
        await selected?.close();
      }
    },
  };
}
