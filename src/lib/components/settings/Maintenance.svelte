<script lang="ts">
  import { onMount } from "svelte";
  import type { AppClient } from "$lib/clients/types";
  import type { MaintenanceStatus } from "$lib/generated/core";
  let { client }: { client: AppClient } = $props();
  let status = $state<MaintenanceStatus | null>(null);
  let busy = $state("");
  let message = $state("");
  let error = $state("");
  let alive = true;
  const previewNote = () =>
    client.mode === "preview" ? " Preview only; no files were changed." : "";

  async function refresh() {
    const result = await client.maintenanceStatus();
    if (alive) status = result;
  }
  async function run(action: "backup" | "rebuild" | "cleanup") {
    if (busy) return;
    busy = action;
    error = "";
    message = "";
    try {
      if (action === "backup") {
        const path = await client.chooseBackupPath();
        if (!path || !alive) return;
        await client.backupDatabase(path);
        if (alive) message = `Database backup saved to ${path}.` + previewNote();
      } else if (action === "rebuild") {
        const report = await client.rebuildIndex();
        if (alive)
          message =
            `Rebuilt search and scanned ${report.scannedFolders} folders. ${report.changedFiles} updated, ${report.removedFiles} removed from the index.` +
            (report.issues.length
              ? ` ${report.issues.length} issues: ${report.issues[0].path}: ${report.issues[0].error}`
              : "") +
            previewNote();
      } else {
        const removed = await client.cleanupCache();
        if (alive) message = `Removed ${removed} unused cached extractions.` + previewNote();
      }
      await refresh();
    } catch (cause) {
      if (alive) error = String(cause);
    } finally {
      if (alive) busy = "";
    }
  }
  onMount(() => {
    void refresh().catch((cause) => {
      if (alive) error = String(cause);
    });
    return () => {
      alive = false;
    };
  });
</script>

<section class="card">
  <h2>Index & Cache</h2>
  <p class="hint">Back up your database, refresh search, or remove unused cached text.</p>
  {#if status}<p class="cache-summary">
      {status.indexedFiles} indexed images · {status.cachedExtractions} cached extractions · {(
        status.cacheBytes / 1024
      ).toFixed(0)} KiB of cached text
    </p>{/if}
  <div class="maintenance-actions" aria-busy={!!busy}>
    <div>
      <div>
        <strong>Database Backup</strong>
        <p>
          Export the index, extraction cache, and queued work. Images and provider settings are
          stored separately. Choose a new filename.
        </p>
      </div>
      <button type="button" onclick={() => run("backup")} disabled={!!busy}
        >{busy === "backup" ? "Exporting…" : "Export Backup"}</button
      >
    </div>
    <div>
      <div>
        <strong>Rebuild Index</strong>
        <p>
          Read your images and their embedded text again, then rebuild search. Unavailable folders
          keep their existing records.
        </p>
      </div>
      <button type="button" onclick={() => run("rebuild")} disabled={!!busy}
        >{busy === "rebuild" ? "Rebuilding…" : "Rebuild Index"}</button
      >
    </div>
    <div>
      <div>
        <strong>Unused Cache</strong>
        <p>
          {status?.unusedCachedExtractions ?? "…"} cached results are no longer referenced by indexed
          images. Clearing them removes that reuse history; text inside your images stays in place.
        </p>
      </div>
      <button
        type="button"
        onclick={() => run("cleanup")}
        disabled={!!busy || !status?.unusedCachedExtractions}
        >{busy === "cleanup" ? "Clearing…" : "Clear Unused Cache"}</button
      >
    </div>
  </div>
  {#if message}<div class="notice success" role="status">{message}</div>{/if}
  {#if error}<div class="notice danger" role="alert">{error}</div>{/if}
</section>

<style>
  .cache-summary {
    margin: 16px 0;
    color: var(--muted, #697e5d);
    font-size: var(--font-size-sm);
  }
  .maintenance-actions > div {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 18px 0;
    border-top: 1px solid var(--border-soft, #e6eae2);
  }
  .maintenance-actions > div > div {
    flex: 1;
  }
  .maintenance-actions strong {
    font-size: var(--font-size-md);
  }
  .maintenance-actions p {
    font-size: var(--font-size-sm);
    color: var(--muted, #697e5d);
    margin: 5px 0 0;
    line-height: 1.65;
  }
  .maintenance-actions button {
    flex-shrink: 0;
  }
  .notice {
    margin-top: 12px;
    margin-bottom: 0;
  }
  @media (max-width: 760px) {
    .maintenance-actions > div {
      align-items: flex-start;
      flex-direction: column;
      gap: 10px;
    }
  }
</style>
