<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { UpdateStatus } from "$lib/clients/types";
  import { version } from "../../../../package.json";

  let {
    status,
    error = "",
    dirty,
    oncheck,
    oninstall,
  }: {
    status: UpdateStatus | null;
    error?: string;
    dirty: boolean;
    oncheck: () => void;
    oninstall: () => void;
  } = $props();
  const busy = $derived(
    status && ["downloading", "installing", "restarting"].includes(status.phase),
  );
  const message = $derived(
    status?.phase === "downloading"
      ? "Downloading and verifying update…"
      : status?.phase === "installing"
        ? "Installing update…"
        : status?.phase === "restarting"
          ? "Update installed. Restarting Lenscribe…"
          : status?.phase === "checking"
            ? "Checking for updates…"
            : status?.available
              ? `Version ${status.available.version} is ready to install`
              : status?.lastChecked
                ? "You’re using the latest version."
                : "Checks automatically while Lenscribe runs.",
  );
  const lastChecked = $derived(
    status?.lastChecked ? new Date(status.lastChecked * 1000).toLocaleString() : "",
  );
</script>

<section class="card app-updates" aria-labelledby="updates-heading">
  <div class="section-heading">
    <div>
      <h2 id="updates-heading" tabindex="-1">App Updates</h2>
      <p class="hint">Lenscribe v{version} · Stable releases</p>
    </div>
    <button
      type="button"
      onclick={oncheck}
      disabled={!status?.supported || busy || status.phase === "checking"}
    >
      <Icon name="retry" size={15} />{status?.phase === "checking"
        ? "Checking…"
        : "Check for Updates"}
    </button>
  </div>
  {#if status && !status.supported}
    <p class="hint">{status.supportMessage}</p>
  {:else}
    <div
      class="update-summary"
      class:available={status?.available !== null && status?.available !== undefined}
    >
      <span class="update-icon"
        ><Icon name={status?.available ? "download" : "check"} size={21} /></span
      >
      <div>
        <strong role="status">{message}</strong>
        <p class="hint">
          {busy
            ? "Lenscribe will restart after installation."
            : lastChecked
              ? `Last checked ${lastChecked}`
              : "Updates are installed when you choose."}
        </p>
      </div>
      {#if status?.available}
        <button
          type="button"
          class="primary"
          onclick={oninstall}
          disabled={busy || dirty || status.phase === "checking"}
        >
          {busy ? "Updating…" : "Update & Restart"}<Icon name="arrow" size={16} />
        </button>
      {/if}
    </div>
    {#if busy}
      <progress
        max="100"
        value={status?.progress ?? undefined}
        aria-label="Update download progress"
      ></progress>
    {/if}
    {#if status?.available?.notes}
      <details class="update-release-notes">
        <summary>What’s new in v{status.available.version}</summary>
        <pre>{status.available.notes}</pre>
      </details>
    {/if}
    {#if status?.available && dirty}<p class="hint">
        Save or discard your settings changes before updating.
      </p>{/if}
    {#if error || status?.error}<p class="connection-feedback danger" role="alert">
        {error || status?.error}
      </p>{/if}
  {/if}
</section>
