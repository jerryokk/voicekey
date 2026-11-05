<script lang="ts">
  import { onMount } from "svelte";
  import { app, disconnect, retry, link, fail } from "../lib/store.svelte";
  import { CH, invoke, hotkeyName, fileSize, type InboxCard } from "../lib/api";
  import Card from "../lib/ui/Card.svelte";
  import StatusDot from "../lib/ui/StatusDot.svelte";

  const remote = $derived(app.remote);
  const last = $derived(app.last);
  const status = $derived(link(remote));
  const failed = $derived(status.dot === "err");
  const ready = $derived(status.dot === "ok");

  /** 「几分钟前」要跟着走：每 30 秒推一下时钟 */
  let now = $state(Date.now());
  onMount(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  let copied = $state(false);
  let busy = $state(false);

  async function copy() {
    if (!remote.url) return;
    try {
      await navigator.clipboard.writeText(remote.url);
      copied = true;
      setTimeout(() => (copied = false), 1200);
    } catch {}
  }

  /** 失败原因由 disconnect 交给 app.error，在页面顶部显示 */
  async function cut() {
    busy = true;
    await disconnect();
    busy = false;
  }

  function ago(ts: number, now: number) {
    const s = Math.max(0, Math.floor(now / 1000 - ts));
    // 时钟 30 秒才走一格，报秒数会失真
    if (s < 60) return "刚刚";
    if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
    return `${Math.floor(s / 3600)} 小时前`;
  }

  function kindName(c: InboxCard) {
    if (c.kind === "clip") return "剪贴板";
    if (c.kind === "text") return "文字";
    return c.files.length === 1 && c.files[0].thumb ? "图片" : "文件";
  }

  function summary(c: InboxCard) {
    if (c.kind !== "files") return c.text.replace(/\s+/g, " ");
    const total = c.files.reduce((n, f) => n + f.size, 0);
    return c.files.length === 1 ? `${c.files[0].name} · ${fileSize(total)}` : `${c.files.length} 个文件 · ${fileSize(total)}`;
  }

  const clock = $derived(
    last ? new Date(last.at * 1000).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" }) : "",
  );
</script>

<h1>连接</h1>
<p class="sub">手机扫码后按住说话，识别结果会打进电脑当前的输入框。</p>

<div class="status">
  <div class="head">
    <StatusDot state={status.dot} pulse={status.dot === "busy"} />
    <b>{status.text}</b>
    {#if app.s}
      <span class="gap"></span>
      {#if app.s.local.enabled}
        <span class="pill local" class:bad={app.local.state === "error"} title={app.local.error ?? ""}>
          本机 · {app.local.state === "error" ? "不可用" : hotkeyName(app.s.local.hotkey)}
        </span>
      {/if}
      <span class="pill">{CH[app.s.channel].short}</span>
    {/if}
  </div>
  <p class="detail">
    {#if failed}
      {remote.error ?? "原因未知"}
    {:else if ready}
      地址每次启动都会变；断开手机后旧二维码立即失效。
    {:else}
      首次约需十秒，请稍候…
    {/if}
  </p>
</div>

{#if ready}
  <Card title="扫码配对" sub="手机相机扫一下，页面会请求一次麦克风权限">
    <div class="qrWrap">
      <div class="qr">{@html remote.qr}</div>
      <div class="side">
        <code class="url">{remote.url}</code>
        <div class="acts">
          <button class="btn" onclick={copy}>{copied ? "已复制" : "复制地址"}</button>
          <button class="btn danger" onclick={cut} disabled={busy}>断开手机</button>
        </div>
        <p class="tip">断开后需重新扫码；适合把手机借给别人之后止损。</p>
      </div>
    </div>
  </Card>
{:else if failed}
  <Card>
    <p class="wait">没有可用的公网地址，手机暂时无法连接。重试会重新拉起服务，地址也会换一个。</p>
    <button class="btn pri" onclick={retry}>重试</button>
  </Card>
{:else}
  <Card>
    <p class="wait"><span class="spin"></span> 正在建立隧道…</p>
  </Card>
{/if}

{#if last}
  <Card title="最近一句">
    <p class="last">{last.text}</p>
    <p class="meta">{clock} · {CH[last.channel].short} · {ago(last.at, now)}</p>
  </Card>
{/if}

{#if app.inbox.length}
  <Card title="最近收到" sub="手机发来的内容，只保留在内存里，退出后清空">
    <ul class="recv">
      {#each app.inbox as c (c.id)}
        <li>
          <span class="kind">{kindName(c)}</span>
          <span class="what" title={summary(c)}>{summary(c)}</span>
          <span class="when">{ago(c.at, now)}</span>
          {#if c.kind === "files"}
            <button class="btn" disabled={c.files.every((f) => f.gone)} onclick={() => invoke("inbox_reveal", { id: c.id }).catch(fail)}>打开位置</button>
          {:else}
            <button class="btn" onclick={() => invoke("inbox_copy", { id: c.id }).catch(fail)}>复制</button>
          {/if}
        </li>
      {/each}
    </ul>
  </Card>
{/if}

<Card title="怎么用">
  <ol class="steps">
    <li><b>扫码</b><span>手机相机对准二维码</span></li>
    <li><b>点好光标</b><span>在电脑上点一下要输入的位置</span></li>
    <li><b>按住说话</b><span>松手后文字自动打进去</span></li>
    <li><b>当触摸板</b><span>手机上半区滑动移光标，轻点单击，双指滚动</span></li>
  </ol>
</Card>

<style>
  .status {
    background: var(--card); border: 1px solid var(--line); border-radius: var(--r-lg);
    padding: 12px 16px; margin-bottom: 12px;
  }
  .head { display: flex; align-items: center; gap: 8px; }
  .head b { font-size: 14px; font-weight: 500; }
  .gap { flex: 1; }
  .pill {
    font-size: var(--fs-tag); color: var(--fg-2);
    border: 1px solid var(--line-2); border-radius: var(--r-pill); padding: 1px 9px;
  }
  .pill.local { color: var(--fg-2); }
  .pill.local + .pill { margin-left: 6px; }
  .pill.local.bad { color: var(--err); border-color: color-mix(in srgb, var(--err) 35%, transparent); }
  .detail { margin: 6px 0 0; font-size: var(--fs-sub); color: var(--fg-2); line-height: 1.7; }
  .recv { list-style: none; padding: 4px 0 12px; display: flex; flex-direction: column; }
  .recv li { display: flex; align-items: center; gap: 10px; padding: 7px 0; border-top: 1px solid var(--line); font-size: var(--fs-sub); }
  .recv li:first-child { border-top: 0; }
  .recv .kind { flex: none; width: 44px; color: var(--fg-3); font-size: var(--fs-tag); }
  .recv .what { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .recv .when { flex: none; color: var(--fg-3); font-size: var(--fs-tag); }
  .recv .btn { min-height: 26px; padding: 3px 10px; font-size: var(--fs-tag); }

  .qrWrap { display: flex; gap: 18px; padding: 10px 0 14px; align-items: center; }
  .qr { background: #fff; border: 1px solid var(--line); border-radius: var(--r-md); padding: 10px; flex: none; }
  .qr :global(svg) { width: 158px; height: 158px; display: block; }
  .side { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 10px; }
  .url {
    font-size: var(--fs-tag); color: var(--fg-2); background: var(--card-2);
    border: 1px solid var(--line); border-radius: var(--r-sm); padding: 7px 10px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .acts { display: flex; gap: 8px; }
  .tip { margin: 0; font-size: var(--fs-tag); color: var(--fg-3); line-height: 1.6; }

  .wait { display: flex; align-items: center; gap: 8px; color: var(--fg-2); font-size: var(--fs-sub); margin: 14px 0; }
  .last { margin: 4px 0 0; font-size: 13.5px; line-height: 1.7; }
  .meta { margin: 6px 0 14px; font-size: var(--fs-tag); color: var(--fg-3); }

  .steps { list-style: none; display: flex; gap: 8px; padding: 2px 0 14px; margin: 0; }
  .steps li { flex: 1; min-width: 0; border: 1px solid var(--line); border-radius: var(--r-md); padding: 10px 12px; }
  .steps b { display: block; font-size: var(--fs-sub); font-weight: 500; }
  .steps span { display: block; margin-top: 3px; font-size: var(--fs-tag); color: var(--fg-3); line-height: 1.6; }
</style>
