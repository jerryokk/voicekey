<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { app, init } from "./lib/store.svelte";
  import Logo from "./lib/Logo.svelte";
  import Connect from "./pages/Connect.svelte";
  import Recog from "./pages/Recog.svelte";
  import Output from "./pages/Output.svelte";
  import General from "./pages/General.svelte";

  const PAGES = [
    { id: "connect", name: "手机接入", icon: '<rect x="6" y="2" width="12" height="20" rx="2.5"/><path d="M10.5 18.5h3"/>' },
    { id: "recog", name: "识别", icon: '<path d="M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM5 11a7 7 0 0 0 14 0M12 18v3"/>' },
    { id: "output", name: "输出", icon: '<path d="M4 6h16M4 12h10M4 18h7M17 15l3 3-3 3"/>' },
    { id: "general", name: "通用", icon: '<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1"/>' },
  ];
  let page = $state("connect");
  const win = getCurrentWindow();

  onMount(() => {
    init();
  });
</script>

{#if app.s && app.info}
  <div class="shell">
    <nav class="side">
      <div class="drag" data-tauri-drag-region></div>
      <div class="brand">
        <div class="app"><Logo /></div>
        <div><b>VoiceKey</b><small>v{app.info.version}</small></div>
      </div>
      {#each PAGES as p}
        <button class="nav" class:on={page === p.id} onclick={() => (page = p.id)}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{@html p.icon}</svg>
          {p.name}
        </button>
      {/each}
      <div class="foot">
        {#if app.paused}
          <span class="dot bad"></span><span>已暂停监听</span>
        {:else if app.remote.url}
          <span class="dot"></span><span>手机可连接</span>
        {:else}
          <span class="dot bad"></span><span>正在建立连接</span>
        {/if}
      </div>
    </nav>
    <div class="wctl">
      <button onclick={() => win.minimize()} aria-label="最小化">
        <svg viewBox="0 0 10 10"><path d="M2 5h6" /></svg>
      </button>
      <button class="x" onclick={() => win.close()} aria-label="关闭">
        <svg viewBox="0 0 10 10"><path d="M2 2l6 6M8 2l-6 6" /></svg>
      </button>
    </div>
    <main>
      <div class="drag" data-tauri-drag-region></div>
      {#if page === "connect"}<Connect />
      {:else if page === "recog"}<Recog />
      {:else if page === "output"}<Output />
      {:else}<General />{/if}
    </main>
  </div>
{/if}

<style>
  .shell { display: flex; height: 100vh; }
  .side { width: 196px; background: var(--side); padding: 0 10px 12px; display: flex; flex-direction: column; gap: 2px; border-right: 1px solid var(--line); }
  .drag { height: 34px; flex: none; }
  .wctl { position: fixed; top: 0; right: 0; height: 34px; display: flex; align-items: center; padding-right: 6px; z-index: 20; }
  .wctl button { width: 34px; height: 26px; display: grid; place-items: center; border: 0; border-radius: 6px;
                 background: none; color: var(--fg2); cursor: default; }
  .wctl button:hover { background: var(--line); color: var(--fg); }
  .wctl button.x:hover { background: #e81123; color: #fff; }
  .wctl svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 1.2; stroke-linecap: round; }
  .brand { display: flex; align-items: center; gap: 10px; padding: 6px 8px 14px; }
  .brand .app { width: 34px; height: 34px; border-radius: 9px; background: #171a2e; display: grid; place-items: center; box-shadow: 0 2px 6px rgba(0, 0, 0, 0.2); }
  .brand .app :global(svg) { width: 26px; height: 26px; }
  .brand b { font-size: 14px; display: block; }
  .brand small { color: var(--fg3); font-size: 11px; }
  .nav { display: flex; align-items: center; gap: 10px; padding: 7px 10px; border-radius: var(--rs); cursor: pointer; color: var(--fg2); border: 0; background: none; text-align: left; width: 100%; position: relative; }
  .nav svg { width: 16px; height: 16px; flex: none; }
  .nav:hover { background: var(--line); }
  .nav.on { background: var(--card); color: var(--fg); box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06); }
  :global(.win) .nav.on::before { content: ""; position: absolute; left: 0; top: 9px; bottom: 9px; width: 3px; border-radius: 2px; background: var(--accent); }
  .foot { margin-top: auto; padding: 10px; border-radius: var(--rs); background: var(--card); font-size: 11px; color: var(--fg2); display: flex; gap: 8px; align-items: center; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--ok); flex: none; }
  .dot.bad { background: var(--err); }
  main { flex: 1; overflow: auto; padding: 0 28px 28px; }
  main :global(h2) { font-size: 20px; font-weight: 650; margin: 0 0 4px; }
  main :global(.sub) { color: var(--fg2); margin-bottom: 18px; }
  main :global(h3) { font-size: 12px; font-weight: 600; color: var(--fg2); margin: 20px 0 8px; letter-spacing: 0.02em; }
</style>
