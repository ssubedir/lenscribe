import { tick } from "svelte";
import type { AppClient } from "$lib/clients/types";
import type { DaemonStatus, Settings } from "$lib/generated/core";
import { validateSettings } from "./validation";
import { rememberConnection } from "./providers";
import type { Inspection, Page } from "./navigation";

export class SettingsController {
  page = $state<Page>("overview");
  status = $state<DaemonStatus | null>(null);
  draft = $state<Settings | null>(null);
  busy = $state<"save" | "retry" | "update" | null>(null);
  error = $state("");
  connectionError = $state("");
  message = $state("");
  advancedOpen = $state(false);
  revealKey = $state(false);
  fileTool = $state("grep");
  inspecting = $state<Inspection | null>(null);
  client = $state.raw<AppClient | null>(null);
  dirty = $derived(
    this.draft !== null && JSON.stringify(this.draft) !== JSON.stringify(this.status?.settings),
  );
  private version = 0;
  private alive = true;
  private cleanup: (() => void) | undefined;
  private timer: ReturnType<typeof setInterval> | undefined;

  start(client: AppClient) {
    this.client = client;
    this.alive = true;
    void this.refresh();
    if (client.pollInterval)
      this.timer = setInterval(() => void this.refresh(), client.pollInterval);
    void client
      .onError((error) => {
        if (this.alive) this.error = error;
      })
      .then((remove) => {
        if (this.alive) this.cleanup = remove;
        else remove();
      })
      .catch((cause) => {
        if (this.alive) this.connectionError = String(cause);
      });
  }

  destroy() {
    this.alive = false;
    this.version++;
    clearInterval(this.timer);
    this.cleanup?.();
    void this.client?.dispose?.().catch(() => {});
  }
  navigate = (page: Page) => {
    if (this.busy === "update") return;
    this.inspecting = null;
    this.page = page;
    this.revealKey = false;
  };
  inspect = (inspection: Inspection) => {
    if (this.busy === "update") return;
    this.page = "folders";
    this.inspecting = inspection;
  };
  clearFeedback = () => {
    this.message = "";
    this.error = "";
  };
  discard = () => {
    if (this.busy) return;
    if (this.status) this.draft = structuredClone($state.snapshot(this.status.settings));
    this.error = "";
    this.revealKey = false;
    this.message = "Changes discarded.";
  };

  updateInstalling = (installing: boolean) => {
    this.version++;
    this.busy = installing ? "update" : null;
  };

  refresh = async () => {
    if (this.busy || !this.client) return;
    const version = ++this.version;
    try {
      const next = await this.client.status();
      if (!this.alive || version !== this.version) return;
      this.accept(next, this.dirty);
      this.connectionError = "";
    } catch (cause) {
      if (this.alive && version === this.version) this.connectionError = String(cause);
    }
  };

  private accept(next: DaemonStatus, keepDraft = false) {
    this.status = next;
    if (!keepDraft) this.draft = structuredClone(next.settings);
  }

  save = async (event?: SubmitEvent) => {
    event?.preventDefault();
    if (!this.draft || !this.client || this.busy) return;
    this.clearFeedback();
    const invalid = validateSettings(this.draft);
    if (invalid) {
      this.page = invalid.page;
      this.error = invalid.message;
      if (invalid.advanced) this.advancedOpen = true;
      await tick();
      document.getElementById(invalid.field)?.focus();
      return;
    }
    this.busy = "save";
    this.version++;
    try {
      const settings = $state.snapshot(this.draft);
      rememberConnection(settings);
      settings.folders.forEach(
        (folder) =>
          (folder.exclusions = folder.exclusions.map((pattern) => pattern.trim()).filter(Boolean)),
      );
      const next = await this.client.save(settings);
      if (!this.alive) return;
      this.accept(next);
      this.message =
        this.client.mode === "preview"
          ? "Preview updated. No desktop settings were changed."
          : next.issues.length
            ? "Settings saved. Some connections need attention."
            : "Settings saved and applied.";
    } catch (cause) {
      if (this.alive) this.error = String(cause);
    } finally {
      if (this.alive) this.busy = null;
    }
  };

  retry = async () => {
    if (this.busy || !this.client) return;
    this.busy = "retry";
    this.version++;
    this.clearFeedback();
    try {
      const next = await this.client.retry();
      if (!this.alive) return;
      this.accept(next, this.dirty);
      this.message = "Saved connections and extraction retried.";
    } catch (cause) {
      if (this.alive) this.error = String(cause);
    } finally {
      if (this.alive) this.busy = null;
    }
  };

  chooseFolder = async () => {
    if (!this.draft || !this.client || this.busy) return;
    try {
      const selected = await this.client.chooseFolder();
      if (selected === "") {
        await this.addFolder();
        return;
      }
      if (!selected || !this.alive || !this.draft) return;
      const normalize = (path: string) =>
        path.replaceAll("\\", "/").replace(/\/$/, "").toLowerCase();
      if (this.draft.folders.some((folder) => normalize(folder.path) === normalize(selected)))
        this.error = "That folder is already in the list.";
      else {
        this.draft.folders.push({ path: selected, enabled: true, exclusions: [], maxImageMib: 0 });
        this.clearFeedback();
      }
    } catch (cause) {
      if (this.alive) this.error = String(cause);
    }
  };

  addFolder = async () => {
    if (!this.draft || this.busy) return;
    this.draft.folders.push({ path: "", enabled: true, exclusions: [], maxImageMib: 0 });
    this.message = "";
    await tick();
    document.getElementById("folder-" + (this.draft.folders.length - 1))?.focus();
  };

  copyCommand = async (command: string) => {
    try {
      await navigator.clipboard.writeText(command);
      this.message = "Command copied.";
    } catch {
      this.error = "Could not copy. Select the command below to copy it manually.";
    }
  };
}
