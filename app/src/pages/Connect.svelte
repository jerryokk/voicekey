<script lang="ts">
  import { app, setPaused } from "../lib/store.svelte";

  const remote = $derived(app.remote);
  let copied = $state(false);

  async function copy() {
    if (!remote.url) return;
    try {
      await navigator.clipboard.writeText(remote.url);
      copied = true;
      setTimeout(() => (copied = false), 1200);
    } catch {
      // 剪贴板不可用时给出地址让人手动抄
    }
  }
</script>

<h2>手机接入</h2>
<div class="sub">手机扫码打开页面，按住说话，识别结果会打进电脑当前的输入框。</div>

{#if remote.qr && !remote.lanOnly}
  <div class="qr">{@html remote.qr}</div>
  <div class="urlrow">
    <code>{remote.url}</code>
    <button class="btn" onclick={copy}>{copied ? "已复制" : "复制"}</button>
  </div>
{:else if remote.state === "failed" || remote.lanOnly}
  <div class="fail">
    <div class="fail-title">{remote.lanOnly ? "公网隧道没有建立" : "手机接入启动失败"}</div>
    <p>
      {#if remote.lanOnly}
        可能是网络连不上 Cloudflare（部分企业网、代理会拦截）。检查网络后重启应用再试；
        电脑与手机在同一局域网时也可以用局域网地址，但手机浏览器要求 https，通常仍需要隧道。
      {:else}
        {remote.error || "未知错误"}。修复后重启应用即可重试。
      {/if}
    </p>
  </div>
{:else}
  <div class="pending">
    <span class="spin"></span>
    正在申请公网地址，首次约需十秒…
  </div>
{/if}

<div class="group">
  <div class="row">
    <div class="lbl">暂停监听<small>暂停后手机怎么按都不会开始识别</small></div>
    <button class="sw" class:on={!app.paused} onclick={() => setPaused(!app.paused)} aria-label="暂停监听"></button>
  </div>
</div>

<h3>怎么用</h3>
<div class="group tips">
  <p>1. 手机扫码，点「启用麦克风」，浏览器会问一次权限。</p>
  <p>2. 在电脑上点好要输入的窗口，让光标留在那里。</p>
  <p>3. 回到手机按住按钮说话，松手后文字自动打进去。</p>
  <p class="warn">地址每次启动都会变，而且谁能打开这个地址谁就能往你的电脑输入文字，别把它发给别人。</p>
</div>

<style>
  .qr { display: grid; place-items: center; padding: 16px; background: #fff; border-radius: 14px; border: 1px solid var(--line); width: fit-content; }
  .qr :global(svg) { width: 220px; height: 220px; display: block; }
  :global(.dark) .qr { background: #fff; }
  .urlrow { display: flex; align-items: center; gap: 8px; margin: 12px 0 4px; }
  .urlrow code { flex: 1; font-size: 11.5px; color: var(--fg2); background: var(--card); border: 1px solid var(--line); border-radius: var(--rs); padding: 8px 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pending { display: flex; align-items: center; gap: 10px; color: var(--fg2); font-size: 13px; padding: 24px 0; }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--line); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .fail { padding: 18px; background: #fdf3f0; border: 1px solid #eccfc5; border-radius: 14px; }
  .fail-title { font-size: 13.5px; font-weight: 600; color: #a04424; margin-bottom: 6px; }
  .fail p { margin: 0; font-size: 12.5px; color: #8a6255; line-height: 1.7; }
  .tips p { margin: 0 0 8px; font-size: 12.5px; color: var(--fg2); line-height: 1.7; }
  .tips .warn { color: var(--fg3); }
</style>
