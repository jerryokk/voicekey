<script lang="ts">
  let { state = "idle", pulse = false }: { state?: "idle" | "ok" | "warn" | "err" | "busy"; pulse?: boolean } = $props();
</script>

<span class="dot {state}" class:pulse></span>

<style>
  .dot {
    display: inline-block; width: 8px; height: 8px; flex: none;
    border-radius: 50%; background: var(--fg-3);
  }
  .dot.ok { background: var(--ok); }
  .dot.warn { background: var(--warn); }
  .dot.err { background: var(--err); }
  .dot.busy { background: var(--accent); }
  .pulse { animation: pulse 1.2s ease-in-out infinite; }
</style>
