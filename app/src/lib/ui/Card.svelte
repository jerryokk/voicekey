<script lang="ts">
  import type { Snippet } from "svelte";
  let { title = "", sub = "", children }: { title?: string; sub?: string; children: Snippet } = $props();
</script>

<section class="card">
  {#if title}
    <header>
      <h3>{title}</h3>
      {#if sub}<p>{sub}</p>{/if}
    </header>
  {/if}
  {@render children?.()}
</section>

<style>
  .card {
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    padding: 4px 16px;
  }
  header { padding: 14px 0 6px; }
  header h3 { margin: 0; }
  header p { margin: 3px 0 0; font-size: var(--fs-sub); color: var(--fg-3); line-height: 1.6; }
</style>
