<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { app, init, save, retry, disconnect, link } from "./lib/store.svelte";
  import { invoke, CH, LOCAL, type Channel } from "./lib/api";
  import { GEAR } from "./lib/icons";
  import ChIcon from "./lib/ChIcon.svelte";
  import StatusDot from "./lib/ui/StatusDot.svelte";

  const win = getCurrentWindow();
  const hide = () => win.hide();

  onMount(() => {
    init();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && hide();
    addEventListener("keydown", esc);
    return () => removeEventListener("keydown", esc);
  });

  const r = $derived(app.remote);
  const status = $derived(link(r));
  const channels = $derived(app.info?.channels ?? []);
  const model = (c: Channel) => app.models[c]?.state ?? "missing";

  /** 切到本地渠道时后端会开始下载；已经选着它、上次下载失败了，再点一下就重试 */
  function pick(c: Channel) {
    if (app.s?.channel !== c) return save({ channel: c });
    if (LOCAL[c] && (model(c) === "missing" || model(c) === "failed")) invoke("model_download", { ch: c });
  }

  function badge(c: Channel): string | null {
    if (!LOCAL[c]) return null;
    const m = app.models[c];
    if (m?.state === "downloading") return `${Math.round((m.progress ?? 0) * 100)}%`;
    if (m?.state === "ready") return null;
    return m?.state === "failed" ? "失败" : "需下载";
  }
</script>

<header data-tauri-drag-region>
  <span class="title" data-tauri-drag-region>手机接入</span>
  <button class="icon" onclick={() => invoke("open_settings")} aria-label="打开设置" title="打开设置">
    <svg class="gear" viewBox="0 0 24 24">{@html GEAR}</svg>
  </button>
  <button class="icon x" onclick={hide} aria-label="关闭" title="关闭">
    <svg viewBox="0 0 16 16"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
  </button>
</header>

<main>
  <section class="scan">
    <div class="code" class:dim={r.phase !== "ready"}>
      {#if r.phase === "ready" && r.qr}
        {@html r.qr}
      {:else if r.phase === "failed"}
        <div class="fail" role="alert">
          <b>{status.text}</b>
          <p>{r.error ?? "原因未知"}</p>
          <button class="btn pri" onclick={retry}>重试</button>
        </div>
      {:else}
        <div class="wait"><span class="spin"></span>{status.text}…</div>
      {/if}
    </div>

    <div class="status" aria-live="polite">
      {#if app.error}
        <span class="error" role="alert">{app.error}</span>
      {:else if r.phase !== "ready"}
        <span class="muted">{r.phase === "failed" ? "手机暂时无法连接" : "首次约需十秒"}</span>
      {:else if r.devices > 0}
        <StatusDot state="ok" /><span>{status.text}</span>
        <button class="link" onclick={disconnect} title="断开所有手机，并换一张新二维码">断开</button>
      {:else}
        <StatusDot /><span class="muted">用手机相机扫码，按住说话</span>
      {/if}
    </div>
  </section>

  <section class="chs">
    <h3>识别渠道</h3>
    <div class="grid" role="radiogroup" aria-label="识别渠道">
      {#each channels as c}
        {@const on = app.s?.channel === c}
        {@const b = badge(c)}
        <button class="ch" class:on role="radio" aria-checked={on} aria-label={CH[c].name} onclick={() => pick(c)} title={CH[c].desc}>
          <ChIcon ch={c} size="sm" />
          <span class="name">{CH[c].short}</span>
          {#if b}<span class="badge" class:err={model(c) === "failed"}>{b}</span>{/if}
        </button>
      {/each}
    </div>
  </section>
</main>

<style>
  header {
    height: 40px; display: flex; align-items: center; gap: 2px; padding: 0 6px 0 16px;
    background: var(--bg-side); border-bottom: 1px solid var(--line);
  }
  .title { flex: 1; font-size: var(--fs-sub); color: var(--fg-2); font-weight: 500; }
  .icon {
    width: 30px; height: 26px; display: grid; place-items: center; border: 0; border-radius: 6px;
    background: none; color: var(--fg-2);
  }
  .icon:hover { background: var(--card-2); color: var(--fg); }
  .icon.x:hover { background: #e81123; color: #fff; }
  .icon svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.3; stroke-linecap: round; }
  .icon svg.gear { stroke-width: 2; stroke-linejoin: round; }

  main { height: calc(100vh - 40px); display: flex; flex-direction: column; gap: 14px; padding: 16px; }

  .scan {
    display: flex; flex-direction: column; align-items: center; gap: 10px;
    background: var(--card); border: 1px solid var(--line); border-radius: var(--r-lg); padding: 16px 16px 12px;
  }
  /* 二维码底色永远是白：深色主题下反相的码有些相机认不出 */
  .code {
    width: 200px; height: 200px; display: grid; place-items: center;
    background: #fff; border-radius: var(--r-md); padding: 10px;
    box-shadow: inset 0 0 0 1px rgba(24, 22, 16, 0.08);
  }
  .code.dim { background: var(--card-2); box-shadow: inset 0 0 0 1px var(--line); }
  .code :global(svg) { width: 180px; height: 180px; display: block; }
  .wait { display: flex; align-items: center; gap: 8px; font-size: var(--fs-sub); color: var(--fg-2); }
  .fail { display: flex; flex-direction: column; align-items: center; gap: 6px; text-align: center; padding: 0 6px; }
  .fail b { font-size: var(--fs-body); font-weight: 500; color: var(--err); }
  .fail p {
    font-size: var(--fs-tag); color: var(--fg-2); line-height: 1.6; max-height: 72px; overflow: auto;
    user-select: text; -webkit-user-select: text; word-break: break-all;
  }
  .fail .btn { margin-top: 4px; }

  .status { height: 22px; display: flex; align-items: center; gap: 7px; font-size: var(--fs-sub); }
  .link {
    margin-left: 4px; border: 0; background: none; padding: 1px 4px; border-radius: 4px;
    color: var(--fg-3); font-size: var(--fs-sub); cursor: pointer;
  }
  .link:hover { color: var(--err); background: var(--err-soft); }
  .error { color: var(--err); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .chs h3 { margin: 0 2px 8px; }
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 7px; }
  .ch {
    position: relative; height: 62px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 5px;
    background: var(--card); border: 1px solid var(--line); border-radius: var(--r-md);
    cursor: pointer; transition: border-color .12s, background .12s;
  }
  .ch:hover { border-color: var(--line-2); }
  .ch:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .ch.on { border-color: var(--accent); background: var(--accent-soft); }
  .name { font-size: var(--fs-tag); color: var(--fg-2); max-width: 92%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ch.on .name { color: var(--fg); font-weight: 500; }
  .badge {
    position: absolute; top: 4px; right: 4px; font-size: 10px; line-height: 14px; padding: 0 5px;
    color: var(--warn); background: color-mix(in srgb, var(--warn) 12%, transparent); border-radius: var(--r-pill);
  }
  .badge.err { color: var(--err); background: var(--err-soft); }
</style>
