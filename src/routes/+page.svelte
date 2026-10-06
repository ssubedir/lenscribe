<script lang="ts">
  import { onMount, tick } from "svelte";
  import { isTauri } from "@tauri-apps/api/core";
  import { createDesktopClient } from "$lib/clients/desktop";
  import { SettingsController } from "$lib/settings/controller.svelte";
  import { pages, type Page } from "$lib/settings/navigation";
  import Icon from "$lib/components/Icon.svelte";
  import TitleBar from "$lib/components/TitleBar.svelte";
  import FileInspector from "$lib/components/FileInspector.svelte";
  import Overview from "$lib/components/settings/Overview.svelte";
  import WatchedFolders from "$lib/components/settings/WatchedFolders.svelte";
  import AIExtraction from "$lib/components/settings/AIExtraction.svelte";
  import SearchRead from "$lib/components/settings/SearchRead.svelte";
  import General from "$lib/components/settings/General.svelte";
  import { version as appVersion } from "../../package.json";
  import "$lib/theme.css";
  import "$lib/styles/base.css";
  import "$lib/styles/controls.css";
  import "$lib/styles/settings.css";

  const model = new SettingsController();
  let desktop = $state(true);
  let preview = $state(false);
  let inspector = $state<{ requestLeave: (leave?: () => void) => void }>();
  const activePage = $derived(pages.find((entry) => entry.id === model.page)!);

  function navigate(page: Page, focus?: string) {
    const leave = () => {
      model.navigate(page);
      if (focus)
        void tick().then(() => {
          const element = document.getElementById(focus);
          element?.scrollIntoView({ block: "center" });
          element?.focus({ preventScroll: true });
        });
    };
    if (model.inspecting && inspector) inspector.requestLeave(leave);
    else leave();
  }
  function closeInspection() {
    model.navigate("folders");
    void tick().then(() => document.getElementById("page-title")?.focus());
  }

  $effect(() => {
    const theme = model.draft?.theme ?? model.status?.settings.theme;
    if (theme) document.documentElement.dataset.theme = theme;
  });
  $effect(() => {
    const theme = model.status?.settings.theme;
    if (desktop && !preview && theme) {
      try {
        localStorage.setItem("lenscribe-theme", theme);
      } catch {
        /* Settings remain the source of truth. */
      }
    }
  });
  onMount(() => {
    desktop = isTauri();
    preview =
      import.meta.env.DEV && !desktop && new URLSearchParams(window.location.search).has("preview");
    let disposed = false;
    if (desktop) model.start(createDesktopClient());
    else if (preview)
      void import("$lib/clients/preview").then(({ createPreviewClient }) => {
        if (!disposed) model.start(createPreviewClient());
      });
    return () => {
      disposed = true;
      model.destroy();
    };
  });
</script>

<svelte:window
  oncontextmenu={(event) => {
    if (desktop || preview) event.preventDefault();
  }}
/>

<svelte:head><title>Lenscribe · {model.inspecting?.name ?? activePage.label}</title></svelte:head>

<div class="settings-ui">
  <div class="app-frame" class:custom-titlebar={desktop || preview}>
    {#if desktop || preview}<TitleBar {preview} onerror={(text) => (model.error = text)} />{/if}
    <div class="app-shell">
      <aside class="sidebar">
        <div class="brand">
          <span class="brand-mark"
            ><img src="/lenscribe-logo-v2.png" alt="" width="46" height="46" /></span
          ><span>Lenscribe<span class="brand-caption">Searchable Images.</span></span>
        </div>
        <p class="nav-caption">WORKSPACE</p>
        <nav aria-label="Settings navigation">
          {#each pages as entry}
            <button
              type="button"
              disabled={model.busy === "update"}
              class:active={model.page === entry.id}
              aria-label={entry.label}
              title={entry.label}
              aria-current={model.page === entry.id ? "page" : undefined}
              onclick={() => navigate(entry.id)}
            >
              <Icon name={entry.icon} size={19} /><span>{entry.label}</span>
              {#if entry.id === "folders" && model.draft?.folders.length}<span class="nav-count"
                  >{model.draft.folders.length}</span
                >{/if}
            </button>
          {/each}
        </nav>
        <div class="sidebar-bottom">
          {#if model.update?.available}
            <button
              type="button"
              class="update-notice"
              aria-label="Update available"
              disabled={model.busy === "update"}
              onclick={() => navigate("general", "updates-heading")}
              title={`Lenscribe v${model.update.available.version} is available`}
            >
              <Icon name="download" size={16} /><span>Update available</span><Icon
                name="arrow"
                size={14}
              />
            </button>
          {/if}
          <span class="version"><span class="version-brand">LENSCRIBE · </span>v{appVersion}</span>
        </div>
      </aside>
      <div class="workspace">
        <main class:inspecting={model.inspecting !== null}>
          <header class="page-header">
            <div>
              {#if model.inspecting}<button
                  type="button"
                  class="text-button inspection-back"
                  onclick={() => inspector?.requestLeave()}
                  ><Icon name="back" size={16} /> Back to Watched Folders</button
                >{/if}
              <h1 id="page-title" tabindex="-1">{model.inspecting?.name ?? activePage.label}</h1>
              <p class="subtitle">
                {model.inspecting
                  ? "Search filenames and extracted text, or edit an image’s transcription."
                  : activePage.description}
              </p>
            </div>
          </header>
          {#if preview}<div class="preview-notice">
              Design preview · Sample data. Changes apply only to this page.
            </div>{/if}
          {#if model.connectionError}<div class="notice danger" role="alert">
              <Icon name="warning" />
              <div>
                <strong>Could not reach the background process</strong>
                <p>{model.connectionError}</p>
                <button type="button" class="text-button" onclick={model.refresh}>Reconnect</button>
              </div>
            </div>{/if}
          {#if model.error}<div class="notice danger" role="alert">
              <Icon name="warning" /><span>{model.error}</span><button
                class="icon-button"
                type="button"
                aria-label="Dismiss error"
                onclick={() => (model.error = "")}><Icon name="close" size={16} /></button
              >
            </div>{/if}
          {#if model.message}<div class="notice success" role="status">
              <Icon name="check" size={18} /><span>{model.message}</span><button
                class="icon-button"
                type="button"
                aria-label="Dismiss notification"
                onclick={() => (model.message = "")}><Icon name="close" size={16} /></button
              >
            </div>{/if}
          {#if model.draft && model.status && model.client}
            {#if model.inspecting}
              {#key model.inspecting.folderId}
                <FileInspector
                  bind:this={inspector}
                  {...model.inspecting}
                  client={model.client}
                  issues={model.status.extraction.issues}
                  processingAvailable={model.status.settings.extraction.enabled &&
                    !model.status.settings.monitoringPaused}
                  onclose={closeInspection}
                  onchange={() => void model.refresh()}
                />
              {/key}
            {:else}<form
                id="settings-form"
                onsubmit={model.save}
                oninput={model.clearFeedback}
                novalidate
              >
                <fieldset disabled={model.busy !== null}>
                  {#if model.page === "overview"}
                    <Overview
                      status={model.status}
                      onnavigate={model.navigate}
                      onretry={model.retry}
                      retryDisabled={model.busy !== null || preview}
                      retrying={model.busy === "retry"}
                      oninspect={model.inspect}
                    />
                  {:else if model.page === "folders"}
                    <WatchedFolders
                      bind:draft={model.draft}
                      status={model.status}
                      onchoose={model.chooseFolder}
                      onadd={model.addFolder}
                      oninspect={model.inspect}
                      onchange={model.clearFeedback}
                    />
                  {:else if model.page === "extraction"}
                    <AIExtraction
                      bind:draft={model.draft}
                      bind:advancedOpen={model.advancedOpen}
                      bind:revealKey={model.revealKey}
                      client={model.client}
                    />
                  {:else if model.page === "api"}
                    <SearchRead
                      bind:draft={model.draft}
                      status={model.status}
                      copyCommand={model.copyCommand}
                      bind:fileTool={model.fileTool}
                    />
                  {:else if model.page === "general"}
                    <General
                      bind:draft={model.draft}
                      client={model.client}
                      update={model.update}
                      updateError={model.updateError}
                      dirty={model.dirty}
                      onCheckUpdate={model.checkUpdate}
                      onInstallUpdate={model.installUpdate}
                    />
                  {/if}
                </fieldset>
              </form>{/if}
          {:else if !desktop}<section class="card empty-state">
              <Icon name="settings" size={36} />
              <h2>Open Lenscribe on your desktop</h2>
              <p>The settings window connects to the app’s background process.</p>
              {#if import.meta.env.DEV}<a class="preview-link" href="/?preview"
                  >Open design preview <Icon name="arrow" size={16} /></a
                >{/if}
            </section>
          {:else if !model.connectionError}<div class="loading-state" role="status">
              Connecting to Lenscribe…
            </div>{/if}
        </main>
        {#if !model.inspecting && model.draft && (model.dirty || model.busy === "save")}<footer
            class="save-bar"
          >
            <div>
              <span class="dot" class:quiet={!model.dirty}></span><span
                >{model.busy === "save" ? "Saving changes…" : "You have unsaved changes"}</span
              >
            </div>
            <div class="save-actions">
              {#if model.dirty}<button
                  type="button"
                  class="text-button"
                  onclick={model.discard}
                  disabled={model.busy !== null}>Discard</button
                >{/if}<button
                type="button"
                onclick={() => void model.save()}
                class="primary"
                disabled={!model.dirty || model.busy !== null}
                >{model.busy === "save" ? "Saving…" : "Save changes"}<Icon
                  name="check"
                  size={16}
                /></button
              >
            </div>
          </footer>{/if}
      </div>
    </div>
  </div>
</div>
