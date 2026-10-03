<script lang="ts">
  import { onMount, untrack } from "svelte";
  import Icon from "./Icon.svelte";
  import type { FileRecord, FileDetails } from "$lib/generated/core";
  import type { AppClient } from "$lib/clients/types";

  let {
    folderId,
    name,
    enabled,
    client,
    initialPath = "",
    issues = [],
    processingAvailable = true,
    onclose,
    onchange,
  }: {
    folderId: number;
    name: string;
    enabled: boolean;
    client: AppClient;
    initialPath?: string;
    issues?: { relativePath: string; error: string; folderId: number }[];
    processingAvailable?: boolean;
    onclose: () => void;
    onchange: () => void;
  } = $props();
  let dialog: HTMLDialogElement;
  let files = $state<FileRecord[]>([]);
  let total = $state(0);
  let query = $state(untrack(() => initialPath));
  let offset = $state(0);
  let selected = $state<FileDetails | null>(null);
  let image = $state("");
  let imageError = $state("");
  let loading = $state(true);
  let working = $state(false);
  let editing = $state(false);
  let text = $state("");
  let error = $state("");
  let notice = $state("");
  let noticeElement = $state<HTMLDivElement>();
  let confirmClose = $state(false);
  const textDirty = $derived(
    editing && (selected?.text === null || text !== (selected?.text ?? "")),
  );
  const selectedIssue = $derived(
    issues.find(
      (issue) => issue.folderId === folderId && issue.relativePath === selected?.relativePath,
    ),
  );
  let generation = 0;
  let listGeneration = 0;
  let alive = true;
  $effect(() => {
    if (notice && noticeElement) noticeElement.parentElement?.scrollTo({ top: 0 });
  });

  async function select(file: FileRecord, quiet = false) {
    if (textDirty || working) return;
    const request = ++generation;
    if (!quiet) error = "";
    if (!quiet || selected?.id !== file.id) notice = "";
    editing = false;
    image = "";
    imageError = "";
    try {
      const details = await client.fileDetails(file.id);
      if (!alive || request !== generation || textDirty) return;
      selected = details;
      text = details.text ?? "";
      try {
        const result = await client.filePreview(file.id);
        if (alive && request === generation) image = result;
      } catch (cause) {
        if (alive && request === generation) imageError = String(cause);
      }
    } catch (cause) {
      if (alive && request === generation) {
        selected = null;
        error = String(cause);
      }
    }
  }

  async function load(reset = false, quiet = false) {
    if (textDirty || working) return;
    if (reset) offset = 0;
    loading = true;
    const request = ++listGeneration;
    try {
      const result = await client.listFiles(folderId, query, offset);
      if (!alive || request !== listGeneration || textDirty || working) return;
      files = result.files;
      total = result.total;
      const next = files.find((file) => file.id === selected?.id) ?? files[0];
      if (next) {
        if (!selected || selected.id !== next.id || selected.recordHash !== next.recordHash)
          await select(next, quiet);
      } else {
        selected = null;
        image = "";
        editing = false;
        notice = "";
      }
    } catch (cause) {
      if (alive) error = String(cause);
    } finally {
      if (alive && request === listGeneration) loading = false;
    }
  }

  async function saveText() {
    if (!selected || working) return;
    working = true;
    error = "";
    try {
      const result = await client.editFile(selected, text);
      const updated = result.file;
      if (!alive) return;
      selected = updated;
      editing = false;
      notice = result.message;
      onchange();
    } catch (cause) {
      if (alive) error = String(cause);
    } finally {
      if (alive) working = false;
    }
    await load(false, true);
  }

  async function queue(force: boolean) {
    if (!selected || working) return;
    working = true;
    error = "";
    try {
      const message = await client.queueFile(selected, force, processingAvailable);
      if (!alive) return;
      notice = message;
      onchange();
    } catch (cause) {
      if (alive) error = String(cause);
    } finally {
      if (alive) working = false;
    }
  }

  function close() {
    if (working) return;
    if (textDirty) confirmClose = true;
    else onclose();
  }
  onMount(() => {
    dialog.showModal();
    void load();
    const timer = setInterval(() => {
      if (!loading && !working && !editing) void load(false, true);
    }, 3000);
    return () => {
      alive = false;
      generation++;
      clearInterval(timer);
    };
  });
</script>

<dialog
  class="inspector-ui"
  bind:this={dialog}
  aria-labelledby="inspector-title"
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
>
  <header>
    <div>
      <span class="eyebrow">FILE INSPECTOR</span>
      <h2 id="inspector-title">{name}</h2>
    </div>
    <button class="icon-button" aria-label="Close file inspector" onclick={close} disabled={working}
      ><Icon name="close" /></button
    >
  </header>
  {#if confirmClose}<div class="notice warning" role="alert">
      <span>Discard your unsaved text changes?</span><button onclick={() => (confirmClose = false)}
        >Keep Editing</button
      ><button onclick={onclose}>Discard & Close</button>
    </div>{/if}
  {#if error}<div class="notice warning" role="alert">{error}</div>{/if}
  <div class="inspector-body">
    <aside class="file-list">
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void load(true);
        }}
      >
        <label for="filename-filter">Find a file</label>
        <div class="filter-row">
          <input
            id="filename-filter"
            type="search"
            bind:value={query}
            placeholder="Filter filenames…"
            disabled={textDirty || working}
          /><button aria-label="Filter files" disabled={textDirty || working}
            ><Icon name="search" size={16} /></button
          >
        </div>
      </form>
      <div class="list-meta">
        <span>{total} {total === 1 ? "image" : "images"}</span><button
          class="icon-button"
          aria-label="Refresh files"
          onclick={() => load()}
          disabled={loading || textDirty || working}><Icon name="retry" size={15} /></button
        >
      </div>
      <div class="file-buttons" aria-label="Indexed images">
        {#each files as file}<button
            class:active={selected?.id === file.id}
            onclick={() => select(file)}
            disabled={textDirty || working}
            ><Icon name="image" size={17} /><span
              ><strong>{file.relativePath.split("/").pop()}</strong><small
                >{file.relativePath.includes("/")
                  ? file.relativePath
                  : file.processor
                    ? "Processed"
                    : "Awaiting extraction"}</small
              ></span
            >{#if issues.some((issue) => issue.folderId === folderId && issue.relativePath === file.relativePath)}<Icon
                name="warning"
                size={15}
              />{/if}</button
          >{:else}<p class="empty">{loading ? "Loading images…" : "No matching images."}</p>{/each}
      </div>
      {#if total > 50}<div class="pagination">
          <button
            onclick={() => {
              offset -= 50;
              void load();
            }}
            disabled={offset === 0 || textDirty || working}>Previous</button
          ><span>{offset + 1}–{Math.min(offset + 50, total)}</span><button
            onclick={() => {
              offset += 50;
              void load();
            }}
            disabled={offset + 50 >= total || textDirty || working}>Next</button
          >
        </div>{/if}
    </aside>
    <section class="file-detail" aria-label="Selected image">
      {#if selected}<div class="detail-heading">
          <div>
            <h3>{selected.relativePath.split("/").pop()}</h3>
            <p>{selected.relativePath} · {(selected.imageLength / 1024).toFixed(0)} KiB</p>
          </div>
          <span class="status-pill">{selected.processor ? "Processed" : "Awaiting extraction"}</span
          >
        </div>
        {#if notice}<div bind:this={noticeElement} class="notice action-notice" role="status">
            <span class="notice-icon"><Icon name="check" size={17} /></span><span
              class="notice-text">{notice}</span
            ><button class="icon-button" aria-label="Dismiss notice" onclick={() => (notice = "")}
              ><Icon name="close" size={16} /></button
            >
          </div>{/if}
        {#if selectedIssue}<div class="notice warning" role="alert">{selectedIssue.error}</div>{/if}
        <div class="preview-image">
          {#if image}<img
              src={image}
              alt={"Preview of " + selected.relativePath}
              onerror={() => {
                image = "";
                imageError = "This image could not be displayed.";
              }}
            />{:else}<div class="preview-placeholder">
              <Icon name="image" size={30} />
              <p>{imageError || "Loading preview…"}</p>
            </div>{/if}
        </div>
        <div class="text-heading">
          <h3>Extracted Text</h3>
          {#if !editing}<button
              onclick={() => {
                editing = true;
                text = selected?.text ?? "";
                notice = "";
              }}
              disabled={!enabled || working}>Edit Text</button
            >{/if}
        </div>
        {#if editing}<label class="sr-only" for="image-text">Image text</label><textarea
            id="image-text"
            bind:value={text}
            rows="8"
            disabled={working}></textarea>
          <div class="edit-actions">
            <span>Saved directly inside this image.</span><button
              onclick={() => {
                editing = false;
                text = selected?.text ?? "";
              }}>Discard Text Changes</button
            ><button class="primary" onclick={saveText} disabled={!textDirty || working}
              >{working ? "Saving…" : "Save Text"}</button
            >
          </div>
        {:else}<pre class="extracted-text">{selected.text === null
              ? "This image is awaiting extraction."
              : selected.text === ""
                ? "Processed successfully; no readable text found."
                : selected.text}</pre>{/if}
        <div class="processing-actions">
          <p>
            {!enabled
              ? "Enable this watched folder to edit or process its images."
              : "Reprocess asks the current model for a fresh result."}
          </p>
          <button
            onclick={() => queue(false)}
            disabled={!enabled || working || editing || selected.processor !== null}
            ><Icon name="retry" size={15} /> Retry</button
          ><button onclick={() => queue(true)} disabled={!enabled || working || editing}
            ><Icon name="spark" size={15} /> Reprocess</button
          >
        </div>
        <details>
          <summary>File Details</summary>
          <dl>
            <dt>Image SHA-256</dt>
            <dd>{selected.imageHash}</dd>
            <dt>Processor</dt>
            <dd>{selected.processor ?? "None"}</dd>
          </dl>
        </details>
      {:else}<div class="empty-detail">
          <Icon name="image" size={36} />
          <p>Select an image to inspect its text.</p>
        </div>{/if}
    </section>
  </div>
</dialog>

<style>
  dialog {
    --control-font-size: var(--font-size-sm);
    --control-font-weight: 400;
    --control-padding: 7px 11px;
    --disabled-opacity: 0.5;
    --field-padding: 9px 10px;
    --field-border-color: var(--border, #dce4db);
    padding: 0;
    width: min(1100px, calc(100vw - 44px));
    max-width: none;
    height: min(780px, calc(100dvh - 76px));
    max-height: none;
    border: 1px solid var(--border, #dce4db);
    border-radius: 12px;
    background: var(--surface, white);
    color: var(--text, #253b35);
    box-shadow: 0 24px 90px #0005;
    overflow: hidden;
  }
  dialog[open] {
    display: flex;
    flex-direction: column;
  }
  dialog::backdrop {
    background: #0e1b146b;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 18px;
    padding: 20px 24px;
    border-bottom: 1px solid var(--border, #e2e7dd);
  }
  h2,
  h3,
  p {
    margin: 0;
  }
  h2 {
    font-size: 20px;
  }
  h3 {
    font-size: var(--font-size-lg);
  }
  .eyebrow {
    font-size: var(--font-size-xs);
    letter-spacing: 1.4px;
    color: var(--subtle, #7a897f);
  }
  .inspector-body {
    display: flex;
    min-height: 0;
    flex: 1;
  }
  .file-list {
    width: 245px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding: 18px 12px;
    background: var(--surface-soft, #f5f6f1);
    border-right: 1px solid var(--border, #e2e7dd);
  }
  form label {
    display: block;
    font-size: var(--font-size-sm);
    margin-bottom: 7px;
    color: var(--muted, #637568);
  }
  .filter-row {
    display: flex;
    align-items: center;
    height: 44px;
    border: 1px solid var(--border, #dce4db);
    border-radius: 8px;
    background: var(--field-bg, #fdfefb);
  }
  .filter-row:focus-within {
    border-color: var(--focus, #2d806b);
    box-shadow: 0 0 0 1px var(--focus, #2d806b);
  }
  .filter-row input {
    flex: 1;
    width: 0;
    height: 100%;
    border: 0;
    border-radius: 8px;
    background: transparent;
    appearance: none;
  }
  .filter-row input:focus-visible {
    outline: none;
  }
  .filter-row button {
    flex: 0 0 32px;
    height: 32px;
    margin-right: 3px;
    padding: 0;
    border: 0;
    border-radius: 5px;
    background: transparent;
  }
  .filter-row button:hover:not(:disabled) {
    background: var(--surface-hover, #eff4ed);
  }
  .filter-row button:focus-visible {
    outline-offset: -3px;
  }
  .list-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin: 12px 3px 7px;
    font-size: var(--font-size-xs);
    color: var(--muted, #637568);
  }
  .file-buttons {
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
  .file-buttons button {
    width: 100%;
    justify-content: flex-start;
    text-align: left;
    padding: 12px 9px;
    border-color: transparent;
    background: transparent;
    margin-bottom: 3px;
  }
  .file-buttons button.active {
    background: var(--nav-active, #dce8dc);
    color: var(--nav-text, #2a634f);
  }
  .file-buttons button > span {
    min-width: 0;
    flex: 1;
  }
  .file-buttons strong {
    display: block;
    font-weight: 550;
    overflow-wrap: anywhere;
  }
  .file-buttons small {
    display: block;
    font-size: var(--font-size-xs);
    font-weight: 400;
    margin-top: 3px;
    color: var(--muted, #637568);
    overflow-wrap: anywhere;
  }
  .pagination {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
    font-size: var(--font-size-xs);
    margin-top: 12px;
  }
  .pagination button {
    font-size: var(--font-size-xs);
    padding: 5px;
  }
  .file-detail {
    flex: 1;
    min-width: 0;
    padding: 22px;
    overflow-y: auto;
  }
  .detail-heading {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    justify-content: space-between;
  }
  .detail-heading p {
    margin-top: 5px;
    font-size: var(--font-size-xs);
    color: var(--muted, #637568);
    overflow-wrap: anywhere;
  }
  .status-pill {
    white-space: nowrap;
    font-size: var(--font-size-xs);
    padding: 4px 7px;
    background: var(--accent-soft, #eef5e9);
    border: 1px solid var(--border-accent, #d5e5d2);
    border-radius: 5px;
    color: var(--accent-text, #527344);
  }
  .preview-image {
    margin: 18px 0;
    display: grid;
    place-items: center;
    min-height: 180px;
    padding: 20px;
    border: 1px solid var(--border, #dce4db);
    border-radius: 8px;
    background: var(--canvas, #fafbf9);
  }
  .preview-image img {
    max-width: 100%;
    max-height: 280px;
    object-fit: contain;
  }
  .preview-placeholder {
    text-align: center;
    color: var(--muted, #637568);
    font-size: var(--font-size-sm);
  }
  .preview-placeholder p {
    margin-top: 8px;
  }
  .text-heading {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 10px;
  }
  .extracted-text {
    min-height: 70px;
    margin: 0;
    padding: 15px;
    border: 1px solid var(--border, #dce4db);
    border-radius: 6px;
    background: var(--field-bg, #fdfefb);
    color: var(--text-secondary, #3c5346);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font:
      var(--font-size-md)/1.85 Consolas,
      monospace;
  }
  textarea {
    resize: vertical;
    font:
      var(--font-size-md)/1.85 Consolas,
      monospace;
  }
  .edit-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 9px;
    align-items: center;
    margin-top: 10px;
  }
  .edit-actions > span {
    flex: 1;
    font-size: var(--font-size-xs);
    color: var(--muted, #637568);
  }
  .processing-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 17px 0;
    border-bottom: 1px solid var(--border-soft, #eef1e9);
  }
  .processing-actions p {
    flex: 1;
    min-width: 130px;
    font-size: var(--font-size-xs);
    color: var(--muted, #637568);
  }
  details {
    margin-top: 13px;
    font-size: var(--font-size-sm);
    color: var(--muted, #637568);
  }
  dl {
    display: grid;
    grid-template-columns: 95px 1fr;
    gap: 8px;
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
    font:
      var(--font-size-xs)/1.7 Consolas,
      monospace;
  }
  .notice {
    border: 0;
    flex-shrink: 0;
    margin: 12px 16px;
    padding: 12px 14px;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    border-radius: 8px;
    background: var(--accent-soft, #eef5e9);
    color: var(--accent-text, #57794a);
    font-size: var(--font-size-md);
    line-height: 1.55;
  }
  .notice.warning {
    background: var(--warning-bg, #fff9ed);
    color: var(--warning, #956e3b);
  }
  .file-detail .notice {
    margin: 16px 0;
  }
  .action-notice {
    flex-wrap: nowrap;
    border: 1px solid var(--border-accent, #d5e5d2);
  }
  .notice-icon {
    display: flex;
    flex-shrink: 0;
  }
  .notice-text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .action-notice .icon-button {
    flex-shrink: 0;
    color: inherit;
  }
  .empty {
    padding: 20px 8px;
    font-size: var(--font-size-sm);
    color: var(--muted, #637568);
  }
  .empty-detail {
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 12px;
    height: 100%;
    color: var(--muted, #637568);
    font-size: var(--font-size-md);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  @media (max-width: 760px) {
    dialog {
      width: calc(100vw - 24px);
    }
    .file-list {
      width: 170px;
    }
    .file-detail {
      padding: 15px;
    }
    header {
      padding: 15px 18px;
    }
    .detail-heading {
      flex-wrap: wrap;
    }
    .preview-image {
      padding: 14px;
    }
  }
</style>
