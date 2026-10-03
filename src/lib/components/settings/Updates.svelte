<script lang="ts">
  import { onDestroy } from "svelte";
  import type { AppClient, UpdateInfo } from "$lib/clients/types";
  let {
    client,
    dirty,
    onInstalling,
  }: { client: AppClient | null; dirty: boolean; onInstalling: (installing: boolean) => void } =
    $props();
  let available = $state<UpdateInfo | null>(null);
  let busy = $state<"check" | "install" | null>(null);
  let message = $state("");
  let failed = $state(false);
  let progress = $state<number | null>(null);
  let alive = true;
  onDestroy(() => {
    alive = false;
  });
  async function check() {
    if (!client || busy) return;
    busy = "check";
    message = "";
    available = null;
    failed = false;
    try {
      const result = await client.checkUpdate();
      if (!alive) return;
      available = result;
      if (!result) message = "You’re using the latest version.";
    } catch (error) {
      if (alive) {
        available = null;
        message = String(error);
        failed = true;
      }
    } finally {
      if (alive) busy = null;
    }
  }
  async function install() {
    if (!client || busy || dirty || !available) return;
    busy = "install";
    progress = null;
    onInstalling(true);
    message = "Downloading and verifying update…";
    failed = false;
    try {
      await client.installUpdate((percent) => {
        if (alive) progress = percent;
      });
      if (alive)
        message =
          client.mode === "preview"
            ? "Preview complete. No update was installed."
            : "Update installed. Restarting Lenscribe…";
    } catch (error) {
      if (alive) {
        available = null;
        message = String(error);
        failed = true;
      }
    } finally {
      onInstalling(false);
      if (alive) busy = null;
    }
  }
</script>

<section class="card">
  <div class="section-heading">
    <h2>App Updates</h2>
    <button type="button" disabled={!client || busy !== null} onclick={check}
      >{busy === "check" ? "Checking…" : "Check for Updates"}</button
    >
  </div>
  <p class="hint">Check GitHub releases for a new version of Lenscribe.</p>
  {#if available}
    <div class="update-details">
      <strong>Lenscribe v{available.version} is available</strong>
      {#if available.notes}<details>
          <summary>Release notes</summary>
          <pre>{available.notes}</pre>
        </details>{/if}
      <button type="button" class="primary" disabled={busy !== null || dirty} onclick={install}
        >{busy === "install" ? "Installing…" : "Download & Install"}</button
      >
      <p class="hint">
        Lenscribe restarts after installation. Save or discard your settings changes first.
      </p>
    </div>
  {/if}
  {#if busy === "install"}<progress
      max="100"
      value={progress ?? undefined}
      aria-label="Update download progress"
    ></progress>{/if}
  {#if message}<p
      class:danger={failed}
      class="connection-feedback"
      role={failed ? "alert" : "status"}
    >
      {message}
    </p>{/if}
</section>
