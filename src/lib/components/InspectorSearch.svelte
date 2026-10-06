<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    value,
    disabled = false,
    notice = null,
    oninput,
    onsubmit,
    oncompositionstart,
    oncompositionend,
  }: {
    value: string;
    disabled?: boolean;
    notice?: string | null;
    oninput: (value: string) => void;
    onsubmit: () => void;
    oncompositionstart: () => void;
    oncompositionend: (value: string) => void;
  } = $props();
  const inputId = $props.id();
  let input: HTMLInputElement;

  onMount(() => input.focus({ preventScroll: true }));
</script>

<form
  class="inspector-search"
  role="search"
  aria-label="Search images"
  autocomplete="off"
  onsubmit={(event) => {
    event.preventDefault();
    onsubmit();
  }}
>
  <label for={inputId}>
    <span class="label-icon" aria-hidden="true"><Icon name="image" size={18} /></span>
    <span>Find an image</span>
  </label>
  <div class="search-field" class:disabled>
    <span class="search-icon" aria-hidden="true"><Icon name="search" size={18} /></span>
    <input
      id={inputId}
      bind:this={input}
      type="search"
      autocomplete="off"
      {value}
      {disabled}
      oninput={(event) => oninput(event.currentTarget.value)}
      {oncompositionstart}
      oncompositionend={(event) => oncompositionend(event.currentTarget.value)}
      placeholder="Search names and extracted text…"
    />
  </div>
  {#if notice}<p class="search-notice" role="status">{notice}</p>{/if}
</form>

<style>
  .inspector-search {
    display: grid;
    grid-template-columns: minmax(160px, 1fr) minmax(0, 640px);
    align-items: center;
    gap: 12px 24px;
    width: 100%;
    flex-shrink: 0;
    padding: 16px 20px;
    border: 1px solid var(--border, #dce4db);
    border-bottom: 0;
    border-radius: 9px 9px 0 0;
    background: var(--surface-soft, #f5f7f3);
  }
  label {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: var(--font-size-xl);
    font-weight: 600;
    color: var(--text, #253b35);
  }
  .label-icon {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background: var(--accent-soft, #e7eee2);
    color: var(--accent-text, #527344);
  }
  .search-field {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    height: 44px;
    padding: 0 14px;
    border: 1px solid var(--border, #dce4db);
    border-radius: 8px;
    background: var(--field-bg, white);
    transition:
      border-color 0.15s,
      box-shadow 0.15s;
  }
  .search-field:hover:not(.disabled) {
    border-color: var(--field-border-focus, #7aa188);
  }
  .search-field:focus-within {
    border-color: var(--focus, #2d806b);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--focus, #2d806b) 14%, transparent);
  }
  .search-field input[type="search"] {
    flex: 1;
    width: 0;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    border-radius: 0;
    outline: none;
    box-shadow: none;
    background: transparent;
    color: var(--text, #253b35);
    font-size: var(--font-size-md);
    appearance: none;
  }
  input::placeholder {
    color: var(--subtle, #7a897f);
    opacity: 1;
  }
  .search-field.disabled {
    opacity: 0.55;
  }
  input:disabled {
    cursor: not-allowed;
  }
  .search-icon {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--muted, #637568);
    pointer-events: none;
  }
  .search-field:focus-within .search-icon {
    color: var(--accent-text, #2f725c);
  }
  .search-notice {
    grid-column: 1 / -1;
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: 1.5;
    color: var(--muted, #637568);
  }
  @media (max-width: 900px) {
    .inspector-search {
      grid-template-columns: minmax(0, 1fr);
      padding: 16px;
    }
  }
</style>
