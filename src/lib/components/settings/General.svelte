<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { Settings } from "$lib/generated/core";
  import type { AppClient } from "$lib/clients/types";
  import Maintenance from "./Maintenance.svelte";
  let { draft = $bindable(), client }: { draft: Settings; client: AppClient } = $props();
</script>

<section class="card">
  <h2>Appearance</h2>
  <p class="hint">Preview a theme, then save to keep it for future launches.</p>
  <div class="theme-options" role="radiogroup" aria-label="Color Theme">
    {#each ["light", "dark"] as theme}
      <label class="theme-option" class:selected={draft.theme === theme}>
        <input type="radio" name="theme" value={theme} bind:group={draft.theme} />
        <span class="theme-sample" class:dark={theme === "dark"} aria-hidden="true"
          ><span class="sample-sidebar"><i></i><i></i><i></i></span><span class="sample-content"
            ><span class="sample-heading"></span><span class="sample-panels"
              ><i></i><i></i><i></i></span
            ><span class="sample-line"></span></span
          ></span
        >
        <span class="theme-option-label"
          ><Icon name={theme === "light" ? "sun" : "moon"} size={17} /><span
            >{theme === "light" ? "Light" : "Dark"}</span
          ><span class="theme-check" aria-hidden="true"
            >{#if draft.theme === theme}<Icon name="check" size={13} />{/if}</span
          ></span
        >
      </label>
    {/each}
  </div>
</section>
<section class="card">
  <h2>Background behavior</h2>
  <label class="switch setting-toggle"
    ><span
      ><strong>Pause monitoring</strong><span
        >Stop watching folders and pause the extraction queue.</span
      ></span
    ><input type="checkbox" bind:checked={draft.monitoringPaused} /><span class="switch-track"
    ></span></label
  ><label class="switch setting-toggle divided"
    ><span
      ><strong>Start at login</strong><span
        >Launch Lenscribe in the system tray when you sign in.</span
      ></span
    ><input type="checkbox" bind:checked={draft.startAtLogin} /><span class="switch-track"
    ></span></label
  ><label class="switch setting-toggle divided"
    ><span
      ><strong>Start with the window hidden</strong><span
        >Lenscribe opens in the system tray when you launch it manually.</span
      ></span
    ><input type="checkbox" bind:checked={draft.startMinimized} /><span class="switch-track"
    ></span></label
  >
  <p class="hint">Resuming scans your folders for images added while paused.</p>
</section>
<Maintenance {client} />
<section class="card">
  <h2>Always there, out of the way</h2>
  <p class="body-copy">
    Closing Settings keeps Lenscribe running. Use the system tray to reopen this window, pause or
    resume monitoring, or quit the app.
  </p>
  <div class="info-note">
    <Icon name="settings" size={18} />
    <p>
      To stop the background process, choose <strong>Quit Lenscribe</strong> in the system tray.
    </p>
  </div>
</section>
