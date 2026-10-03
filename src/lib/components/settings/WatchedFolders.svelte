<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { DaemonStatus, Settings } from "$lib/generated/core";
  import { folderName, type Inspection, type Page } from "$lib/settings/navigation";
  let {
    draft = $bindable(),
    status,
    onchoose,
    onadd,
    oninspect,
    onchange,
  }: {
    draft: Settings;
    status: DaemonStatus;
    onchoose: () => void;
    onadd: () => void;
    oninspect: (inspection: Inspection) => void;
    onchange: () => void;
  } = $props();
  function folderState(folder: DaemonStatus["folderStatuses"][number]) {
    if (!folder.enabled) return "Disabled";
    if (status?.settings.monitoringPaused) return "Paused";
    if (folder.lastError) return "Needs attention";
    return folder.watching ? "Watching" : "Not connected";
  }
</script>

<div class="section-heading page-actions">
  <p class="hint">PNG, JPEG & WebP · Subfolders included</p>
  <button type="button" class="primary" onclick={onchoose}
    ><Icon name="plus" size={17} /> Choose folder</button
  >
</div>
{#each draft.folders as folder, index}
  {@const health = status.folderStatuses.find((entry) => entry.path === folder.path)}
  <section class="card folder-card">
    <div class="section-heading">
      <div class="folder-title">
        <span class="folder-icon"><Icon name="folder" /></span>
        <h2>{folderName(folder.path)}</h2>
      </div>
      <button
        type="button"
        class="icon-button remove"
        aria-label={"Remove folder " + (index + 1)}
        onclick={() => {
          draft!.folders.splice(index, 1);
          onchange();
        }}><Icon name="trash" size={18} /></button
      >
    </div>
    <label class="field" for={"folder-" + index}
      >Folder path<input
        id={"folder-" + index}
        type="text"
        bind:value={folder.path}
        placeholder="C:/Users/me/Pictures"
        spellcheck="false"
      /></label
    >
    <div class="folder-detail-row">
      <label class="switch compact"
        ><input
          type="checkbox"
          bind:checked={folder.enabled}
          aria-label={"Monitor folder " + (index + 1)}
        /><span class="switch-track"></span><span>Monitor this folder</span></label
      ><span class="small-label"
        >{!health || folder.enabled !== health.enabled
          ? "Unsaved configuration"
          : folderState(health)}</span
      >
    </div>
    <details class="folder-rules">
      <summary
        >Folder Rules <span class="hint"
          >{folder.exclusions.filter((pattern) => pattern.trim()).length
            ? folder.exclusions.filter((pattern) => pattern.trim()).length + " exclusions"
            : "Include all images"}</span
        ></summary
      ><label class="field" for={"exclusions-" + index}
        >Exclude patterns<textarea
          id={"exclusions-" + index}
          rows="3"
          value={folder.exclusions.join("\n")}
          oninput={(event) => (folder.exclusions = event.currentTarget.value.split("\n"))}
          placeholder={"temp/**\n*-thumbnail.png"}
          spellcheck="false"></textarea><span class="hint"
          >One per line. Use / for folders, * for names, and ** for subfolders. Bare names match at
          any depth.</span
        ></label
      ><label class="field port-field" for={"size-" + index}
        >Maximum image size (MiB)<input
          id={"size-" + index}
          type="number"
          min="0"
          max="131072"
          step="1"
          bind:value={folder.maxImageMib}
        /><span class="hint"
          >0 includes every size. Based on original image bytes, before appended text.</span
        ></label
      >
      <p class="hint">
        Excluded images leave the index and queue. Their files and existing text stay in place.
      </p>
    </details>
    {#if health?.folderId !== null && health?.folderId !== undefined}<div class="folder-stats">
        <span><strong>{health.imageCount}</strong> indexed</span><span
          ><strong>{health.imageCount - health.pendingImages}</strong> processed</span
        ><span><strong>{health.pendingImages}</strong> pending</span>
      </div>{/if}
    {#if health?.lastError}<p class="inline-error">{health.lastError}</p>{/if}
    {#if health?.rootHash}<details class="merkle-details">
        <summary>Merkle root</summary><code>{health.rootHash}</code>
      </details>{/if}
    {#if health?.folderId !== null && health?.folderId !== undefined}<button
        type="button"
        class="text-button inspect-button"
        onclick={() =>
          oninspect({
            folderId: health.folderId!,
            name: folderName(health.path),
            enabled: health.enabled,
          })}>Inspect Files <Icon name="arrow" size={15} /></button
      >{/if}
  </section>
{:else}<section class="card empty-state">
    <Icon name="folder" size={36} />
    <h2>Where do your images land?</h2>
    <p>Add a screenshots folder, a receipts folder, or any place you save images.</p>
    <button type="button" class="primary" onclick={onchoose}
      ><Icon name="plus" size={17} /> Choose a folder</button
    >
  </section>{/each}
<button type="button" class="text-button" onclick={onadd}
  ><Icon name="plus" size={16} /> Add a folder by path</button
>
<p class="hint section-hint">
  Save to apply folder changes. Removing a folder stops monitoring; its images and existing
  extracted text stay in place.
</p>
