<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { app, init, link } from "./lib/store.svelte";
  import { GEAR } from "./lib/icons";
  import Logo from "./lib/Logo.svelte";
  import StatusDot from "./lib/ui/StatusDot.svelte";
  import Connect from "./pages/Connect.svelte";
  import Channels from "./pages/Channels.svelte";
  import Preferences from "./pages/Preferences.svelte";

  const PAGES = [
    { id: "connect", name: "连接", icon: '<rect x="6" y="2" width="12" height="20" rx="2.5"/><path d="M10.5 18.5h3"/>' },
    { id: "channels", name: "渠道", icon: '<path d="M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM5 11a7 7 0 0 0 14 0M12 18v3"/>' },
    { id: "settings", name: "设置", icon: GEAR },
  ];

  let page = $state("connect");
  const win = getCurrentWindow();

  const status = $derived(link(app.remote));

  onMount(() => {
    init();
  });
</script>

{#if app.s && app.info}
  <div class="shell">
    <aside>
      <div class="drag" data-tauri-drag-region></div>
      <div class="brand">
        <div class="logo"><Logo /></div>
        <div class="who">
          <b>VoiceKey</b>
          <small>v{app.info.version}</small>
        </div>
      </div>

      <nav>
        {#each PAGES as p}
          <button class:on={page === p.id} onclick={() => (page = p.id)}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{@html p.icon}</svg>
            {p.name}
          </button>
        {/each}
      </nav>

      <div class="foot">
        <StatusDot state={status.dot} pulse={status.dot === "busy"} />
        <span>{status.text}</span>
      </div>
    </aside>

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
      {#if app.error}<p class="alert" role="alert">{app.error}</p>{/if}
      {#if page === "connect"}<Connect />
      {:else if page === "channels"}<Channels />
      {:else}<Preferences />{/if}
    </main>
  </div>
{/if}

<style>
  .shell { display: flex; height: 100vh; background: var(--bg); }

  aside {
    width: 178px; flex: none; display: flex; flex-direction: column;
    background: var(--bg-side); border-right: 1px solid var(--line);
    padding: 0 8px 10px;
  }
  .drag { height: 34px; flex: none; }
  .brand { display: flex; align-items: center; gap: 9px; padding: 2px 8px 16px; }
  .logo { width: 28px; height: 28px; flex: none; border-radius: 8px; overflow: hidden; display: grid; place-items: center; }
  .logo :global(svg) { width: 28px; height: 28px; display: block; }
  .who b { display: block; font-size: 13px; font-weight: 500; }
  .who small { display: block; font-size: var(--fs-tag); color: var(--fg-3); }

  nav { display: flex; flex-direction: column; gap: 1px; }
  nav button {
    display: flex; align-items: center; gap: 9px; width: 100%;
    padding: 7px 10px; border: 0; border-radius: var(--r-sm);
    background: none; color: var(--fg-2); font-size: 13px; text-align: left; cursor: pointer;
  }
  nav button:hover { background: var(--card-2); color: var(--fg); }
  /* 点完不留描边（桌面应用的惯例）；键盘 Tab 过来时靠底色提示位置 */
  nav button:focus,
  nav button:focus-visible { outline: none; background: var(--card-2); }
  nav button.on { background: var(--card); color: var(--fg); box-shadow: 0 1px 2px rgba(24, 22, 16, .06); position: relative; }
  nav button.on::before {
    content: ""; position: absolute; left: 0; top: 8px; bottom: 8px; width: 3px;
    border-radius: 2px; background: var(--accent);
  }
  nav svg { width: 15px; height: 15px; flex: none; }

  .foot {
    margin-top: auto; display: flex; align-items: center; gap: 8px;
    padding: 9px 10px; border-radius: var(--r-sm);
    background: var(--card); border: 1px solid var(--line);
    font-size: var(--fs-tag); color: var(--fg-2);
  }

  .wctl { position: fixed; top: 0; right: 0; height: 34px; display: flex; align-items: center; padding-right: 6px; z-index: 20; }
  .wctl button {
    width: 34px; height: 26px; display: grid; place-items: center;
    border: 0; border-radius: 6px; background: none; color: var(--fg-2); cursor: default;
  }
  .wctl button:hover { background: var(--line); color: var(--fg); }
  .wctl button.x:hover { background: #e81123; color: #fff; }
  .wctl svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 1.2; stroke-linecap: round; }

  main { flex: 1; min-width: 0; overflow: auto; padding: 0 24px 24px; }
  main :global(.card) { margin-bottom: 12px; }
  .alert {
    margin-bottom: 12px; padding: 7px 12px; border-radius: var(--r-sm);
    font-size: var(--fs-sub); color: var(--err); background: var(--err-soft);
    user-select: text; -webkit-user-select: text;
  }
</style>
