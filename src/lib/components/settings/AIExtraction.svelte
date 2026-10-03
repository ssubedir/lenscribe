<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import { onDestroy } from "svelte";
  import type { AppClient } from "$lib/clients/types";
  import type { LlmProvider, ModelCatalog, Settings } from "$lib/generated/core";
  import { providerOptions, providerPresets, switchConnection } from "$lib/settings/providers";
  let {
    draft = $bindable(),
    advancedOpen = $bindable(false),
    revealKey = $bindable(false),
    client,
  }: {
    draft: Settings;
    advancedOpen: boolean;
    revealKey: boolean;
    client: AppClient | null;
  } = $props();
  let provider = $derived(providerPresets[draft.extraction.provider]);
  let catalog = $state<ModelCatalog | null>(null);
  let busy = $state(false);
  let feedback = $state("");
  let failed = $state(false);
  let sequence = 0,
    alive = true,
    lastConnection = "";
  const connection = $derived(
    JSON.stringify([draft.extraction.provider, draft.extraction.baseUrl, draft.extraction.apiKey]),
  );
  $effect(() => {
    if (lastConnection !== connection) {
      lastConnection = connection;
      sequence++;
      catalog = null;
      feedback = "";
      busy = false;
    }
  });
  onDestroy(() => {
    alive = false;
    sequence++;
  });

  async function fetchModels() {
    if (!client || busy) return;
    feedback = "";
    failed = false;
    if (provider.requiresKey && !draft.extraction.apiKey.trim()) {
      feedback = "Enter your API key first.";
      failed = true;
      return;
    }
    const version = ++sequence,
      signature = connection;
    busy = true;
    try {
      const settings = $state.snapshot(draft.extraction);
      const result = await client.discoverModels(settings);
      if (!alive || version !== sequence || connection !== signature) return;
      catalog = result;
      feedback = result.models.length
        ? `${result.models.length} models found${result.truncated ? " (catalog limited)" : ""}.`
        : "No matching models were returned. You can enter a model ID manually.";
    } catch (error) {
      if (alive && version === sequence && connection === signature) {
        feedback = String(error);
        failed = true;
      }
    } finally {
      if (alive && version === sequence) busy = false;
    }
  }

  function changeProvider(selected: LlmProvider) {
    switchConnection(draft, selected);
    revealKey = false;
  }
</script>

<section class="card">
  <label class="switch setting-toggle"
    ><span
      ><strong>Automatic text extraction</strong><span
        >Send pending images to your vision model and save the text.</span
      ></span
    ><input type="checkbox" bind:checked={draft.extraction.enabled} /><span class="switch-track"
    ></span></label
  >
</section>
<section class="card">
  <div class="section-heading">
    <h2>Model connection</h2>
  </div>
  <label class="field" for="llm-provider">
    Provider
    <select
      id="llm-provider"
      value={draft.extraction.provider}
      onchange={(event) => changeProvider(event.currentTarget.value as LlmProvider)}
    >
      {#each providerOptions as option}
        <option value={option.id}>{option.label}</option>
      {/each}
    </select>
    <span class="hint">{provider.description}</span>
    <span class="hint"
      >Switching providers restores its saved connection. Changes are kept when you save.</span
    >
  </label>
  {#if provider.editableBaseUrl}
    <label class="field" for="llm-url"
      >Base URL<input
        id="llm-url"
        type="url"
        bind:value={draft.extraction.baseUrl}
        placeholder={provider.baseUrl}
        spellcheck="false"
      /><span class="hint">{provider.urlHint}</span></label
    >
  {/if}
  <div class="connection-actions">
    <button type="button" disabled={!client || busy} onclick={fetchModels}
      >{busy ? "Fetching models…" : "Fetch Models"}</button
    >
  </div>
  {#if catalog?.models.length}
    <label class="field" for="available-model"
      >Available models
      <select
        id="available-model"
        value={catalog.models.some((entry) => entry.id === draft.extraction.model)
          ? draft.extraction.model
          : ""}
        onchange={(event) => {
          draft.extraction.model = event.currentTarget.value;
          feedback = "";
        }}
      >
        <option value="">Choose a model…</option>
        {#each catalog.models as entry}<option value={entry.id}
            >{entry.name} · {entry.id}{entry.vision === null
              ? " (image support unverified)"
              : ""}</option
          >{/each}
      </select>
      <span class="hint">Choose a model that accepts images, or enter its ID below.</span>
    </label>
  {/if}
  <label class="field" for="llm-model"
    >Vision model<input
      id="llm-model"
      type="text"
      bind:value={draft.extraction.model}
      placeholder={draft.extraction.provider === "ollama"
        ? "Installed vision model name and tag"
        : "Your provider’s vision model ID"}
      maxlength="256"
      spellcheck="false"
      oninput={() => {
        feedback = "";
      }}
    /><span class="hint">Use the exact model name from your provider. It must accept images.</span
    ></label
  >
  <label class="field" for="llm-key">API key{provider.requiresKey ? "" : " (optional)"}</label>
  <div class="key-field">
    <input
      id="llm-key"
      type={revealKey ? "text" : "password"}
      bind:value={draft.extraction.apiKey}
      placeholder={provider.requiresKey
        ? "Your provider’s API key"
        : "Leave blank for no authentication"}
      autocomplete="off"
      maxlength="4096"
      spellcheck="false"
    /><button
      type="button"
      class="icon-button"
      aria-label={revealKey ? "Hide API key" : "Show API key"}
      aria-pressed={revealKey}
      onclick={() => (revealKey = !revealKey)}><Icon name="eye" size={18} /></button
    >
  </div>
  <p class="hint">Saved as plain text in your local settings file.</p>
  {#if feedback}<p
      class:danger={failed}
      class="connection-feedback"
      role={failed ? "alert" : "status"}
    >
      {feedback}
    </p>{/if}
</section>
<section class="card">
  <details class="advanced-options" bind:open={advancedOpen}>
    <summary>Extraction options<span class="hint">Prompt, limits & processing speed</span></summary>
    <label class="field" for="llm-prompt"
      >Transcription instructions<textarea
        id="llm-prompt"
        bind:value={draft.extraction.prompt}
        rows="7"
        maxlength="16384"></textarea></label
    >
    <div class="field-grid">
      <label class="field" for="llm-tokens"
        >Output token limit<input
          id="llm-tokens"
          type="number"
          min="1"
          max="32768"
          step="1"
          bind:value={draft.extraction.maxTokens}
        /></label
      ><label class="field" for="llm-timeout"
        >Timeout (seconds)<input
          id="llm-timeout"
          type="number"
          min="1"
          max="600"
          step="1"
          bind:value={draft.extraction.timeoutSeconds}
        /></label
      >
    </div>
    <div class="field-grid">
      <label class="field" for="llm-concurrency"
        >Concurrent images<input
          id="llm-concurrency"
          type="number"
          min="1"
          max="8"
          step="1"
          bind:value={draft.extraction.concurrency}
        /><span class="hint"
          >One at a time by default. Increase to process several images together.</span
        ></label
      >
      <label class="field" for="llm-rate"
        >Requests per minute<input
          id="llm-rate"
          type="number"
          min="0"
          max="600"
          step="1"
          bind:value={draft.extraction.requestsPerMinute}
        /><span class="hint">0 means unlimited. Cached results do not use requests.</span></label
      >
    </div>
    <p class="hint">
      Changes apply to pending images. Matching image hashes and extraction settings reuse saved
      text. Use Inspect Files to reprocess an image with a fresh model request.
    </p>
  </details>
</section>
<div class="info-note">
  <Icon name="spark" size={18} />
  <p>
    Text is appended to the image and added to your search index. Original image bytes are
    preserved.
  </p>
</div>
