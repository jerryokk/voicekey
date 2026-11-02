<script lang="ts">
  let { value = "", options = [] as { v: string; label: string }[], onchange }: {
    value?: string; options?: { v: string; label: string }[]; onchange?: (v: string) => void;
  } = $props();
</script>

<div class="seg">
  {#each options as o}
    <button class:on={value === o.v} onclick={() => onchange?.(o.v)}>{o.label}</button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex; flex: none; padding: 2px;
    background: var(--card-2); border: 1px solid var(--line); border-radius: var(--r-sm);
  }
  button {
    border: 0; background: none; cursor: pointer; color: var(--fg-2);
    padding: 4px 12px; border-radius: 6px; transition: background .12s, color .12s;
  }
  button.on { background: var(--card); color: var(--fg); box-shadow: 0 1px 2px rgba(0, 0, 0, .08); }
</style>
