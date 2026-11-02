<script lang="ts">
  let { label = "", desc = "", children } = $props();
</script>

<div class="row">
  <div class="lbl">
    <div>{label}</div>
    {#if desc}<small>{desc}</small>{/if}
  </div>
  {@render children?.()}
</div>

<style>
  .row {
    display: flex; align-items: center; gap: 16px;
    padding: 12px 0; min-height: 48px;
  }
  .row + :global(*) { border-top: 1px solid var(--line); }
  .lbl { flex: 1; min-width: 0; }
  .lbl small { display: block; margin-top: 2px; font-size: var(--fs-tag); color: var(--fg-3); line-height: 1.6; }
</style>
