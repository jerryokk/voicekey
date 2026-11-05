<script lang="ts">
  import { onMount } from "svelte";
  import { app, save, fail } from "../lib/store.svelte";
  import { invoke, on, HOTKEYS, hotkeyName, type LocalMic, type Mic } from "../lib/api";
  import Card from "../lib/ui/Card.svelte";
  import Row from "../lib/ui/Row.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import StatusDot from "../lib/ui/StatusDot.svelte";

  const local = $derived(app.s!.local);
  const status = $derived(app.local);
  const key = $derived(hotkeyName(local.hotkey));
  const set = (patch: Partial<LocalMic>) => save({ local: { ...local, ...patch } });

  let mics = $state<Mic[]>([]);
  let fallback = $state<string | null>(null);
  let level = $state(0);
  let testing = $state(false);
  let testTimer: ReturnType<typeof setTimeout> | undefined;

  async function loadMics() {
    try {
      const r = await invoke<{ mics: Mic[]; default: string | null }>("list_mics");
      mics = r.mics;
      fallback = r.default;
    } catch (e) {
      mics = [];
      fail(e);
    }
  }

  const defaultName = $derived(mics.find((m) => m.id === fallback)?.name);
  /** 存着的设备拔掉了：下拉里也要能看见它，否则用户以为设置丢了 */
  const missing = $derived(local.device !== null && mics.length > 0 && !mics.some((m) => m.id === local.device));

  function test() {
    testing = true;
    invoke("mic_test");
    clearTimeout(testTimer);
    testTimer = setTimeout(() => ((testing = false), (level = 0)), 5000);
  }

  onMount(() => {
    const un = on<number>("mic-level", (v) => (level = v));
    return () => {
      un.then((f) => f());
      clearTimeout(testTimer);
    };
  });

  $effect(() => {
    // 开启之后才去枚举：没开时不碰音频设备
    if (local.enabled && !mics.length) loadMics();
  });

  const head = $derived.by(() => {
    if (!local.enabled) return { dot: "idle" as const, text: "未开启" };
    if (status.state === "error") return { dot: "err" as const, text: "用不了" };
    if (status.state === "ready") return { dot: "ok" as const, text: `按住${key}说话` };
    return { dot: "busy" as const, text: "正在准备" };
  });

  const BARS = 14;
</script>

<h1>本机</h1>
<p class="sub">手机不在手边时，用电脑自己的麦克风：按住快捷键说话，松开即识别。不经过手机和隧道，打开 VoiceKey 就能用。</p>

<div class="status">
  <div class="head">
    <StatusDot state={head.dot} />
    <b>{head.text}</b>
    <Switch checked={local.enabled} label="用本机麦克风说话" onchange={(v) => set({ enabled: v })} />
  </div>
  <p class="detail">
    {#if !local.enabled}
      开启后，在任何窗口里按住{key}就能说话，识别渠道、边说边上屏都和手机一样。
    {:else if status.state === "error"}
      {status.error}
    {:else}
      按住时按 Esc 取消这一句；轻点一下不会触发。麦克风只在按住时打开，松手即关闭。
    {/if}
  </p>
  {#if local.enabled && status.state === "error"}
    <div class="fix">
      {#if status.privacy}
        <button class="btn pri" onclick={() => invoke("open_mic_privacy").catch(fail)}>打开系统隐私设置</button>
      {/if}
      <button class="btn" onclick={() => invoke("mic_retry").catch(fail)}>重试</button>
    </div>
  {/if}
</div>

{#if local.enabled}
  <Card title="收音">
    <Row label="输入设备" desc={status.note ?? "选「系统默认」会跟着 Windows 的默认麦克风走"}>
      <select
        class="sel"
        aria-label="输入设备"
        value={local.device ?? ""}
        onfocus={loadMics}
        onchange={(e) => set({ device: e.currentTarget.value || null })}
      >
        <option value="">系统默认{defaultName ? `（${defaultName}）` : ""}</option>
        {#each mics as m (m.id)}<option value={m.id}>{m.name}</option>{/each}
        {#if missing}<option value={local.device}>已断开的设备</option>{/if}
      </select>
    </Row>
    <Row label="试一下" desc="对着电脑说几句，看电平条有没有动">
      <div class="test">
        <span class="meter" aria-hidden="true">
          {#each Array(BARS) as _, i}<i class:on={testing && level * BARS > i}></i>{/each}
        </span>
        <button class="btn" onclick={test} disabled={testing || status.state === "error"}>
          {testing ? "正在听…" : "试听 5 秒"}
        </button>
      </div>
    </Row>
  </Card>

  <Card title="快捷键">
    <Row label="按住说话" desc="被选中的键只给 VoiceKey 用，其他程序收不到它；左边的同名键不受影响">
      <Segmented value={local.hotkey} options={HOTKEYS} label="按住说话的快捷键" onchange={(v) => set({ hotkey: v })} />
    </Row>
  </Card>

  <p class="foot">以管理员身份运行的窗口在前台时，快捷键会被 Windows 拦下，这时请用手机。</p>
{/if}

<style>
  .status {
    background: var(--card); border: 1px solid var(--line); border-radius: var(--r-lg);
    padding: 12px 16px; margin-bottom: 12px;
  }
  .head { display: flex; align-items: center; gap: 8px; }
  .head b { flex: 1; font-size: 14px; font-weight: 500; }
  .detail { margin: 6px 0 0; font-size: var(--fs-sub); color: var(--fg-2); line-height: 1.7; }
  .fix { margin-top: 10px; display: flex; gap: 8px; }
  .sel {
    width: 280px; padding: 5px 8px; border-radius: var(--r-sm);
    border: 1px solid var(--line-2); background: var(--card); color: var(--fg);
  }
  .sel:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .test { display: flex; align-items: center; gap: 12px; }
  .meter { display: flex; align-items: center; gap: 3px; height: 16px; }
  .meter i { width: 4px; height: 100%; border-radius: 2px; background: var(--line-2); transition: background .06s; }
  .meter i.on { background: var(--ok); }
  .foot { margin: 0 2px; font-size: var(--fs-tag); color: var(--fg-3); line-height: 1.6; }
</style>
