<script lang="ts">
  import { onMount } from "svelte";
  import { app, init, refreshRemote } from "./lib/store.svelte";

  onMount(() => {
    init();
  });

  const ready = $derived(Boolean(app.remote.qr));
  const failed = $derived(app.remote.state === "failed" || app.remote.lanOnly === true);
</script>

<div class="wrap">
  {#if ready}
    <div class="qr">{@html app.remote.qr}</div>
    <p class="hint">用手机相机扫码，在打开的页面里按住说话</p>
  {:else if failed}
    <div class="fail">
      <b>公网隧道没有建立</b>
      <p>检查网络后重启 VoiceKey；详情见设置里的「手机接入」。</p>
    </div>
  {:else}
    <div class="pending"><span class="spin"></span>正在申请公网地址…</div>
    <p class="hint">首次约需十秒</p>
    <button class="retry" onclick={() => refreshRemote()}>刷新</button>
  {/if}
</div>

<style>
  .wrap { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; padding: 16px 16px; height: 100vh; background: #f7f7f5; }
  .qr { background: #fff; border: 1px solid var(--line); border-radius: 14px; padding: 12px; }
  .qr :global(svg) { width: 240px; height: 240px; display: block; }
  .hint { margin: 0; font-size: 12.5px; color: var(--fg2); text-align: center; line-height: 1.6; }
  .pending { display: flex; align-items: center; gap: 8px; color: var(--fg2); font-size: 13px; margin-top: 60px; }
  .spin { width: 13px; height: 13px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .retry { margin-top: 6px; border: 1px solid var(--line); background: #fff; border-radius: 8px; padding: 6px 14px; font-size: 12.5px; color: var(--fg2); cursor: pointer; }
  .fail { margin-top: 40px; text-align: center; font-size: 12.5px; color: var(--fg2); line-height: 1.7; }
  .fail b { display: block; color: #a04424; font-size: 13px; margin-bottom: 6px; }
  .fail p { margin: 0; }
  :global(.dark) .wrap { background: #1b1b19; }
  :global(.dark) .qr { background: #fff; }
</style>
