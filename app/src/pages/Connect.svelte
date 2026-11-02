<script lang="ts">
  import { app, disconnect, refreshRemote } from "../lib/store.svelte";
  import { CH } from "../lib/api";
  import Card from "../lib/ui/Card.svelte";
  import StatusDot from "../lib/ui/StatusDot.svelte";

  const remote = $derived(app.remote);
  const last = $derived(app.last);
  const failed = $derived(remote.state === "failed" || remote.lanOnly === true);

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

  async function cut() {
    busy = true;
    await disconnect();
    busy = false;
  }

  function ago(ts: number) {
    const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
    if (s < 60) return `${s} 秒前`;
    if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
    return `${Math.floor(s / 3600)} 小时前`;
  }

  const clock = $derived(
    last ? new Date(last.at * 1000).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" }) : "",
  );
</script>

<h1>连接</h1>
<p class="sub">手机扫码后按住说话，识别结果会打进电脑当前的输入框。</p>

<div class="status">
  <div class="head">
    <StatusDot state={failed ? "err" : remote.qr ? "ok" : "busy"} pulse={!failed && !remote.qr} />
    <b>{failed ? (remote.lanOnly ? "公网隧道未建立" : "手机接入启动失败") : remote.qr ? "可以扫码连接" : "正在申请公网地址"}</b>
    {#if app.s}<span class="pill">{CH[app.s.channel].short}</span>{/if}
  </div>
  <p class="detail">
    {#if failed}
      {remote.error ?? (remote.lanOnly ? "公网隧道没有建立，手机暂时连不上。" : "未知错误")}
    {:else if remote.qr}
      地址每次启动都会变；断开手机后旧二维码立即失效。
    {:else}
      首次约需十秒，请稍候…
    {/if}
  </p>
</div>

{#if remote.qr && !failed}
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
{:else if !failed}
  <Card>
    <p class="wait"><span class="spin"></span> 正在建立隧道…</p>
    <button class="btn" onclick={() => refreshRemote()}>刷新</button>
  </Card>
{:else}
  <Card>
    <p class="wait">没有可用的公网地址，手机暂时无法连接。</p>
    <button class="btn" onclick={() => refreshRemote()}>重新检测</button>
  </Card>
{/if}

{#if last}
  <Card title="最近一句">
    <p class="last">{last.text}</p>
    <p class="meta">{clock} · {CH[last.channel].short} · {ago(last.at)}</p>
  </Card>
{/if}

<Card title="怎么用">
  <ol class="steps">
    <li><b>扫码</b><span>手机相机对准二维码</span></li>
    <li><b>点好光标</b><span>在电脑上点一下要输入的位置</span></li>
    <li><b>按住说话</b><span>松手后文字自动打进去</span></li>
  </ol>
</Card>

<style>
  .sub { margin: 4px 0 16px; color: var(--fg-2); }
  .status {
    background: var(--card); border: 1px solid var(--line); border-radius: var(--r-lg);
    padding: 12px 16px; margin-bottom: 12px;
  }
  .head { display: flex; align-items: center; gap: 8px; }
  .head b { font-size: 14px; font-weight: 500; }
  .pill {
    margin-left: auto; font-size: var(--fs-tag); color: var(--fg-2);
    border: 1px solid var(--line-2); border-radius: var(--r-pill); padding: 1px 9px;
  }
  .detail { margin: 6px 0 0; font-size: var(--fs-sub); color: var(--fg-2); line-height: 1.7; }

  .qrWrap { display: flex; gap: 16px; padding: 10px 0 14px; align-items: flex-start; }
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
