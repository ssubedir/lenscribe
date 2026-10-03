<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { DaemonStatus, Settings } from "$lib/generated/core";
  import { folderName, type Inspection, type Page } from "$lib/settings/navigation";
  let {
    status,
    onnavigate,
    onretry,
    oninspect,
    retryDisabled,
    retrying,
  }: {
    status: DaemonStatus;
    onnavigate: (page: Page) => void;
    onretry: () => void;
    oninspect: (inspection: Inspection) => void;
    retryDisabled: boolean;
    retrying: boolean;
  } = $props();
  const percentage = $derived(
    status?.totalImages ? Math.round((status.processedImages / status.totalImages) * 100) : 0,
  );
  const watchedCount = $derived(
    status?.folderStatuses.filter((folder) => folder.watching).length ?? 0,
  );
  const needsAttention = $derived(
    Boolean(
      status &&
      (status.issues.length || status.extraction.failedImages || status.extraction.lastError),
    ),
  );
  const phaseTitle = $derived(
    !status
      ? "Connecting…"
      : status.extraction.phase === "disabled"
        ? "Extraction is off"
        : status.extraction.phase === "paused"
          ? "Processing is paused"
          : status.extraction.phase === "extracting"
            ? "Reading your images"
            : status.extraction.phase === "needsConfiguration"
              ? "Extraction needs attention"
              : status.extraction.failedImages
                ? "Some images need attention"
                : status.pendingImages
                  ? "Waiting to process"
                  : "All caught up",
  );
  const phaseDescription = $derived(
    !status
      ? ""
      : status.extraction.phase === "disabled"
        ? "Enable AI extraction to turn images into searchable text."
        : status.extraction.phase === "paused"
          ? "Resume monitoring in General to continue the queue."
          : status.extraction.phase === "extracting"
            ? `${status.settings.extraction.concurrency === 1 ? "One image at a time." : `Up to ${status.settings.extraction.concurrency} images at a time.`} New arrivals join the queue automatically.`
            : status.extraction.phase === "needsConfiguration"
              ? "Check your endpoint settings, then retry extraction."
              : status.extraction.failedImages
                ? "Failed images stay in the queue. Review the errors below."
                : status.pendingImages
                  ? "Pending images run when retry and request limits allow."
                  : "Lenscribe will process new images as they arrive.",
  );
  function folderState(folder: DaemonStatus["folderStatuses"][number]) {
    if (!folder.enabled) return "Disabled";
    if (status?.settings.monitoringPaused) return "Paused";
    if (folder.lastError) return "Needs attention";
    return folder.watching ? "Watching" : "Not connected";
  }
</script>

<div class="metrics" aria-label="Image processing totals">
  <div class="metric">
    <span class="metric-label"><Icon name="image" size={17} /> Images indexed</span><strong
      >{status.totalImages.toLocaleString()}</strong
    ><span>In enabled folders</span>
  </div>
  <div class="metric">
    <span class="metric-label"><Icon name="check" size={17} /> Processed</span><strong
      >{status.processedImages.toLocaleString()}</strong
    ><span>Text saved to the image</span>
  </div>
  <div class="metric">
    <span class="metric-label"><Icon name="clock" size={17} /> In the queue</span><strong
      >{status.pendingImages.toLocaleString()}</strong
    ><span>Includes images awaiting retry</span>
  </div>
</div>
<section class="card processing-card">
  <div class="card-heading">
    <div class="heading-icon"><Icon name="spark" /></div>
    <div>
      <h2>{phaseTitle}</h2>
      <p>{phaseDescription}</p>
    </div>
    <span class="live-tag">LIVE</span>
  </div>
  {#if status.totalImages}
    <div class="progress-label">
      <span>{status.processedImages} of {status.totalImages} images processed</span><strong
        >{percentage}%</strong
      >
    </div>
    <progress value={status.processedImages} max={status.totalImages} aria-label="Images processed"
    ></progress>
  {/if}
  {#if status.extraction.currentFile}<div class="current-file">
      <span class="activity-dot"></span><span
        >Currently reading <strong>{status.extraction.currentFile}</strong
        >{#if status.extraction.activeFiles.length > 1}
          · {status.extraction.activeFiles.length - 1} more{/if}</span
      >
    </div>{/if}
  {#if !status.settings.folders.length}<button
      type="button"
      class="primary"
      onclick={() => onnavigate("folders")}
      >Add your first folder <Icon name="arrow" size={16} /></button
    >
  {:else if !status.settings.extraction.enabled || status.extraction.phase === "needsConfiguration"}<button
      type="button"
      class="text-button"
      onclick={() => onnavigate("extraction")}
      >Configure extraction <Icon name="arrow" size={16} /></button
    >{/if}
</section>
{#if needsAttention}
  <section class="card attention-card">
    <div class="section-heading">
      <h2><Icon name="warning" size={18} /> Needs attention</h2>
      <button type="button" onclick={onretry} disabled={retryDisabled}
        >{retrying ? "Retrying…" : "Retry saved configuration"}</button
      >
    </div>
    {#each status.issues as issue}<div class="issue">
        <strong>{issue.source}</strong>
        <p>{issue.error}</p>
      </div>{/each}
    {#if status.extraction.lastError}<p class="issue-message">{status.extraction.lastError}</p>{/if}
    {#each status.extraction.issues as issue}<div class="issue">
        <strong>{issue.relativePath}</strong>
        <p>{issue.error}</p>
        <span class="small-label"
          >Attempt {issue.attempts} · {issue.retrying
            ? `Retry ${issue.retryAtMs ? new Date(issue.retryAtMs).toLocaleTimeString() : "scheduled"}`
            : "Manual retry needed"}</span
        ><button
          type="button"
          class="text-button"
          onclick={() => {
            const folder = status!.folderStatuses.find(
              (folder) => folder.folderId === issue.folderId,
            );
            oninspect({
              folderId: issue.folderId,
              name: folder ? folderName(folder.path) : "Images",
              enabled: folder?.enabled ?? false,
              initialPath: issue.relativePath,
            });
          }}>Inspect Image <Icon name="arrow" size={14} /></button
        >
      </div>{/each}
    {#if status.extraction.failedImages}<p class="hint">
        {status.extraction.failedImages} images remain pending. Successful images are kept.
      </p>{/if}
  </section>
{/if}
<section class="card folder-overview">
  <div class="section-heading">
    <h2>Watching your folders</h2>
    <button type="button" class="text-button" onclick={() => onnavigate("folders")}
      >Manage folders <Icon name="arrow" size={16} /></button
    >
  </div>
  {#each status.folderStatuses as folder}<div class="summary-row">
      <span class="folder-icon"><Icon name="folder" /></span>
      <div>
        <strong>{folderName(folder.path)}</strong>
        <p title={folder.path}>{folder.path}</p>
      </div>
      <div class="summary-count">
        <strong>{folder.imageCount} images</strong><span>{folderState(folder)}</span>
      </div>
    </div>
  {:else}<div class="empty-state">
      <Icon name="folder" size={32} />
      <h3>A home for your images</h3>
      <p>
        Choose a folder. Lenscribe watches it and its subfolders for PNG, JPEG, and WebP images.
      </p>
      <button type="button" onclick={() => onnavigate("folders")}
        >Set up folders <Icon name="arrow" size={16} /></button
      >
    </div>{/each}
  {#if status.folderStatuses.length}<p class="card-footnote">
      {watchedCount}
      {watchedCount === 1 ? "folder" : "folders"} actively watched · PNG, JPEG & WebP
    </p>{/if}
</section>
<button type="button" class="api-shortcut" onclick={() => onnavigate("api")}
  ><span class="heading-icon"><Icon name="terminal" /></span><span
    ><strong>Search and read with your usual tools</strong><span
      >Use grep, cat, Get-Content, or the local API.</span
    ></span
  ><Icon name="arrow" size={18} /></button
>
