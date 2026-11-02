<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { app, init, refreshRemote } from "./lib/store.svelte";

  const win = getCurrentWindow();

  onMount(() => {
    init();
  });

  const ready = $derived(Boolean(app.remote.qr));
  const failed = $derived(app.remote.state === "failed" || app.remote.lanOnly === true);
</script>

<div class="bar" data-tauri-drag-region>
  <span>扫码连接手机</span>
  <button class="x" onclick={() => win.close()} aria-label="关闭">
    <svg viewBox="0 0 10 10"><path d="M2 2l6 6M8 2l-6 6" /></svg>
  </button>
</div>

<div class="wrap">
  {#if ready}
    <div class="qr">{@html app.remote.qr}</div>
    <p class="hint">用手机相机扫码，在打开的页面里按住说话</p>
  {:else if failed}
    <div class="fail">
      <b>公网隧道没有建立</b>
      <p>{app.remote.error ?? "手机暂时无法连接。检查网络后重启 VoiceKey，或在「连接」页重新检测。"}</p>
    </div>
  {:else}
    <div class="pending"><span class="spin"></span>正在申请公网地址…</div>
    <p class="hint">首次约需十秒</p>
    <button class="retry" onclick={() => refreshRemote()}>刷新</button>
  {/if}
</div>

<style>
  .bar { height: 34px; display: flex; align-items: center; justify-content: space-between; padding: 0 6px 0 14px;
         font-size: 12.5px; color: var(--fg2); background: var(--bg-side); border-bottom: 1px solid var(--line); }
  .bar .x { width: 30px; height: 24px; display: grid; place-items: center; border: 0; border-radius: 6px;
            background: none; color: var(--fg2); cursor: default; }
  .bar .x:hover { background: #e81123; color: #fff; }
  .bar svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 1.2; stroke-linecap: round; }
  .wrap { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; padding: 16px 16px; height: calc(100vh - 34px); background: var(--bg); }
  .qr { background: #fff; border: 1px solid var(--line); border-radius: 14px; padding: 12px; }
  .qr :global(svg) { width: 240px; height: 240px; display: block; }
  .hint { margin: 0; font-size: 12.5px; color: var(--fg2); text-align: center; line-height: 1.6; }
  .pending { display: flex; align-items: center; gap: 8px; color: var(--fg2); font-size: 13px; margin-top: 60px; }
  .spin { width: 13px; height: 13px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .retry { margin-top: 6px; border: 1px solid var(--line); background: #fff; border-radius: 8px; padding: 6px 14px; font-size: 12.5px; color: var(--fg2); cursor: pointer; }
  .fail { text-align: center; font-size: 12.5px; color: var(--fg2); line-height: 1.7; max-width: 250px; }
  .fail b { display: block; color: #a04424; font-size: 13px; margin-bottom: 6px; }
  .fail p { margin: 0; }
  :global(.dark) .qr { background: #fff; }
</style>
