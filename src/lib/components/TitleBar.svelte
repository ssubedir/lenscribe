<script lang="ts">
  import { onMount } from "svelte";
  import { isTauri } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import Icon from "$lib/components/Icon.svelte";

  let { preview = false, onerror }: { preview?: boolean; onerror: (message: string) => void } =
    $props();
  let maximized = $state(false);
  let focused = $state(true);
  let pending = $state(false);
  let appWindow: ReturnType<typeof getCurrentWindow> | undefined;

  async function control(action: "minimize" | "maximize" | "close") {
    if (pending) return;
    if (preview) {
      if (action === "maximize") maximized = !maximized;
      return;
    }
    if (!appWindow) return;
    pending = true;
    try {
      if (action === "minimize") await appWindow.minimize();
      else if (action === "close") await appWindow.close();
      else {
        await appWindow.toggleMaximize();
        maximized = await appWindow.isMaximized();
      }
    } catch (cause) {
      onerror("Could not " + action + " the settings window: " + String(cause));
    } finally {
      pending = false;
    }
  }

  onMount(() => {
    if (!isTauri()) return;
    const currentWindow = getCurrentWindow();
    appWindow = currentWindow;
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    async function syncMaximized() {
      try {
        const next = await currentWindow.isMaximized();
        if (!disposed) maximized = next;
      } catch (cause) {
        if (!disposed) onerror("Could not read the window state: " + String(cause));
      }
    }
    function subscribe(listener: Promise<UnlistenFn>) {
      void listener
        .then((remove) => {
          if (disposed) remove();
          else unlisteners.push(remove);
        })
        .catch((cause) => {
          if (!disposed) onerror("Could not track the window state: " + String(cause));
        });
    }
    void syncMaximized();
    void currentWindow
      .isFocused()
      .then((next) => {
        if (!disposed) focused = next;
      })
      .catch((cause) => {
        if (!disposed) onerror("Could not read the window focus: " + String(cause));
      });
    subscribe(currentWindow.onResized(() => void syncMaximized()));
    subscribe(
      currentWindow.onFocusChanged(({ payload }) => {
        if (!disposed) focused = payload;
      }),
    );
    return () => {
      disposed = true;
      appWindow = undefined;
      for (const remove of unlisteners) remove();
    };
  });
</script>

<header class="titlebar" class:inactive={!focused} aria-label="Window Title Bar">
  <div class="titlebar-drag" data-tauri-drag-region>
    <div class="titlebar-brand">
      <img src="/lenscribe-logo-v2.png" alt="" width="18" height="18" /><span>Lenscribe</span>
    </div>
  </div>
  <div class="window-controls" role="group" aria-label="Window Controls">
    <button
      type="button"
      aria-label="Minimize Window"
      title="Minimize"
      disabled={pending}
      onclick={() => control("minimize")}><Icon name="minimize" size={15} /></button
    >
    <button
      type="button"
      aria-label={maximized ? "Restore Window" : "Maximize Window"}
      title={maximized ? "Restore" : "Maximize"}
      disabled={pending}
      onclick={() => control("maximize")}
      ><Icon name={maximized ? "restore" : "maximize"} size={14} /></button
    >
    <button
      type="button"
      class="close-window"
      aria-label="Close Settings"
      title="Close Settings"
      disabled={pending}
      onclick={() => control("close")}><Icon name="close" size={16} /></button
    >
  </div>
</header>

<style>
  .titlebar {
    position: fixed;
    inset: 0 0 auto;
    z-index: 20;
    display: flex;
    height: var(--window-titlebar-height, 38px);
    background: var(--chrome, #f4f7f1);
    border-bottom: 1px solid var(--border, #e0e7dc);
    color: var(--text-secondary, #53694e);
    user-select: none;
  }
  .titlebar-drag {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    padding: 0 16px;
    -webkit-app-region: drag;
  }
  .titlebar-brand {
    display: flex;
    align-items: center;
    gap: 8px;
    pointer-events: none;
    font:
      550 var(--font-size-md)/1 "Segoe UI",
      system-ui,
      sans-serif;
  }
  .titlebar-brand img {
    object-fit: contain;
  }
  .inactive .titlebar-brand {
    opacity: 0.65;
  }
  .window-controls {
    display: flex;
    align-items: stretch;
    flex-shrink: 0;
  }
  .window-controls button {
    display: grid;
    place-items: center;
    width: 44px;
    height: 100%;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--muted, #687a60);
    cursor: default;
    transition:
      background 0.12s,
      color 0.12s;
    -webkit-app-region: no-drag;
  }
  .window-controls button:hover {
    background: var(--surface-hover, #e3ebdc);
    color: var(--accent-text, #304a33);
  }
  .window-controls button:focus-visible {
    outline: 2px solid var(--focus, #2d806b);
    outline-offset: -4px;
  }
  .window-controls .close-window:hover {
    background: var(--close-bg, #c84940);
    color: white;
  }
  .window-controls button:disabled {
    opacity: 0.5;
  }
  @media (prefers-reduced-motion: reduce) {
    .window-controls button {
      transition: none;
    }
  }
</style>
