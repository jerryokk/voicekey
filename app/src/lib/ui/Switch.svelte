<script lang="ts">
  let { checked = false, disabled = false, onchange, label = "" }: {
    checked?: boolean; disabled?: boolean; label?: string; onchange?: (v: boolean) => void;
  } = $props();
</script>

<button
  class="sw"
  class:on={checked}
  {disabled}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  onclick={() => onchange?.(!checked)}
></button>

<style>
  .sw {
    width: 36px; height: 20px; flex: none; position: relative;
    border: 0; border-radius: var(--r-pill);
    background: var(--line-2); cursor: pointer; transition: background .14s;
  }
  .sw::after {
    content: ""; position: absolute; left: 2px; top: 2px; width: 16px; height: 16px;
    border-radius: 50%; background: #fff; transition: left .14s;
    box-shadow: 0 1px 2px rgba(0, 0, 0, .25);
  }
  .sw.on { background: var(--accent); }
  .sw.on::after { left: 18px; }
  .sw:disabled { opacity: .4; cursor: not-allowed; }
</style>
