<script lang="ts">
  import { highlightText } from "../search/highlight";
  let { text, query, fuzzy = true }: { text: string; query: string; fuzzy?: boolean } = $props();
  const parts = $derived(highlightText(text, query, fuzzy));
</script>

{#each parts as part}{#if part.matched}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}

<style>
  mark {
    color: var(--highlight-text, #433914);
    background: var(--highlight-bg, #f6e7a5);
    border-radius: 3px;
    box-decoration-break: clone;
    -webkit-box-decoration-break: clone;
  }
</style>
