<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, on, applyTheme, fileSize, type InboxCard, type AppInfo } from "./lib/api";

  /** 剪贴板卡片几秒后自动收起；鼠标停在上面时暂停 */
  const CLIP_SECONDS = 3;

  let cards = $state<InboxCard[]>([]);
  let hovering = $state(false);
  let error = $state<string | null>(null);
  let countdown = $state<Record<number, number>>({});
  let root: HTMLElement;

  const open = $derived(cards.filter((c) => c.open));

  onMount(() => {
    const un = [
      on<InboxCard[]>("inbox", (v) => (cards = v)),
      on<AppInfo["settings"]>("settings", (s) => applyTheme(s.theme)),
    ];
    invoke<AppInfo>("get_state").then((info) => {
      applyTheme(info.settings.theme);
      cards = info.inbox;
    });
    // 内容高度变了：让后端把窗口贴着右下角重摆
    const ro = new ResizeObserver(() => invoke("inbox_resize", { height: Math.ceil(root.scrollHeight) }));
    ro.observe(root);
    const tick = setInterval(() => {
      if (hovering) return;
      for (const c of open) {
        if (c.kind !== "clip") continue;
        const left = (countdown[c.id] ?? CLIP_SECONDS) - 1;
        if (left <= 0) dismiss(c.id);
        else countdown[c.id] = left;
      }
    }, 1000);
    return () => {
      un.forEach((p) => p.then((f) => f()));
      ro.disconnect();
      clearInterval(tick);
    };
  });

  const dismiss = (id: number) => invoke("inbox_dismiss", { id });

  async function act(cmd: "inbox_copy" | "inbox_reveal", id: number) {
    error = null;
    try {
      await invoke(cmd, { id });
    } catch (e) {
      error = String(e);
    }
  }

  const preview = (t: string) => (t.length > 80 ? t.slice(0, 80) + "…" : t);
</script>

<main
  bind:this={root}
  onmouseenter={() => (hovering = true)}
  onmouseleave={() => (hovering = false)}
  aria-live="polite"
>
  {#each open as c (c.id)}
    <section class="card">
      <header>
        <span class="ic" aria-hidden="true">
          {#if c.kind === "clip"}
            <svg viewBox="0 0 24 24"><rect x="8" y="3" width="8" height="4" rx="1" /><path d="M8 5H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2" /><path d="M9 14l2 2 4-4" /></svg>
          {:else if c.kind === "text"}
            <svg viewBox="0 0 24 24"><path d="M4 7V5h16v2M9 19h6M12 5v14" /></svg>
          {:else if c.files.length === 1 && c.files[0].thumb}
            <svg viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="9" cy="10" r="2" /><path d="M21 16l-5-5-9 9" /></svg>
          {:else}
            <svg viewBox="0 0 24 24"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /></svg>
          {/if}
        </span>
        <b>
          {#if c.kind === "clip"}已写入电脑剪贴板
          {:else if c.kind === "text"}收到文字
          {:else if c.files.length === 1 && c.files[0].thumb}收到图片
          {:else if c.of > 1}收到 {c.files.length}{c.files.length < c.of ? ` / ${c.of}` : ""} 个文件
          {:else}收到文件{/if}
        </b>
        <button class="x" onclick={() => dismiss(c.id)} aria-label="关闭">
          <svg viewBox="0 0 16 16"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
        </button>
      </header>

      {#if c.kind === "clip"}
        <p class="quote">“{preview(c.text)}”</p>
        <div class="acts">
          <button class="btn" onclick={() => dismiss(c.id)}>我知道了{hovering ? "" : ` · ${countdown[c.id] ?? CLIP_SECONDS}`}</button>
        </div>
      {:else if c.kind === "text"}
        <pre class="text">{c.text}</pre>
        <div class="acts"><button class="btn pri" onclick={() => act("inbox_copy", c.id)}>复制</button></div>
      {:else}
        {#if c.files.length === 1 && c.files[0].thumb}
          {@const f = c.files[0]}
          <div class="image">
            <img src={f.thumb} alt="" />
            <div class="meta"><span class="name" title={f.name}>{f.name}</span><small>{fileSize(f.size)} · 下载</small></div>
          </div>
        {:else}
          <ul class="files">
            {#each c.files.slice(0, 5) as f}
              <li class:gone={f.gone}><span class="name" title={f.name}>{f.name}</span><small>{f.gone ? "已移走" : fileSize(f.size)}</small></li>
            {/each}
            {#if c.files.length > 5}<li class="more">还有 {c.files.length - 5} 个</li>{/if}
          </ul>
        {/if}
        <div class="acts">
          <button class="btn pri" onclick={() => act("inbox_reveal", c.id)} disabled={c.files.every((f) => f.gone)}>
            在文件夹中显示
          </button>
        </div>
      {/if}
    </section>
  {/each}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</main>

<style>
  main { display: flex; flex-direction: column; gap: 8px; padding: 8px; background: var(--bg); min-height: 0; }
  .card { background: var(--card); border: 1px solid var(--line); border-radius: var(--r-lg); padding: 12px 14px; }
  header { display: flex; align-items: center; gap: 9px; }
  header b { flex: 1; font-size: 13px; font-weight: 500; }
  .ic {
    width: 26px; height: 26px; flex: none; border-radius: 8px; display: grid; place-items: center;
    background: var(--accent-soft); color: var(--accent);
  }
  .ic svg { width: 15px; height: 15px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .x { width: 26px; height: 24px; border: 0; border-radius: 6px; display: grid; place-items: center; background: none; color: var(--fg-3); }
  .x:hover { background: var(--card-2); color: var(--fg); }
  .x svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; }
  .quote { margin-top: 8px; font-size: var(--fs-sub); color: var(--fg-2); line-height: 1.6; word-break: break-all; }
  .text {
    margin-top: 8px; max-height: 96px; overflow: auto; padding: 8px 10px; border-radius: var(--r-sm);
    background: var(--card-2); border: 1px solid var(--line);
    font: 12.5px/1.65 var(--font); white-space: pre-wrap; word-break: break-word;
    user-select: text; -webkit-user-select: text;
  }
  .image { display: flex; align-items: center; gap: 12px; margin-top: 10px; }
  .image img { width: 64px; height: 64px; flex: none; object-fit: cover; border-radius: var(--r-sm); border: 1px solid var(--line); }
  .meta { min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .files { list-style: none; margin-top: 8px; display: flex; flex-direction: column; gap: 2px; }
  .files li { display: flex; gap: 10px; font-size: var(--fs-sub); padding: 3px 0; }
  .files li.gone { opacity: 0.5; }
  .files li.more { color: var(--fg-3); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-sub); }
  small { flex: none; color: var(--fg-3); font-size: var(--fs-tag); font-variant-numeric: tabular-nums; }
  .acts { display: flex; justify-content: flex-end; margin-top: 10px; }
  .err { font-size: var(--fs-tag); color: var(--err); padding: 0 4px; }
</style>
