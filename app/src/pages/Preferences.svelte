<script lang="ts">
  import { app, save } from "../lib/store.svelte";
  import Card from "../lib/ui/Card.svelte";
  import Row from "../lib/ui/Row.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  const s = $derived(app.s!);
  const info = $derived(app.info!);
  const all = $derived(s.channel === "all");

  const THEMES = [
    { v: "system", label: "跟随系统" },
    { v: "light", label: "浅色" },
    { v: "dark", label: "深色" },
  ];
</script>

<h1>设置</h1>
<p class="sub">输出方式、启动行为与外观。</p>

<Card title="输出">
  <Row
    label="边说边上屏"
    desc={all ? "多渠道模式需先挑选结果，此项不可用" : "识别中的文字实时打到光标处；关闭后改为松手时一次性粘贴"}
  >
    <Switch checked={s.streaming} disabled={all} label="边说边上屏" onchange={(v) => save({ streaming: v })} />
  </Row>
  <Row label="胶囊显示实时文字" desc="在屏幕底部的胶囊里预览识别内容">
    <Switch checked={s.liveText} label="胶囊显示实时文字" onchange={(v) => save({ liveText: v })} />
  </Row>
</Card>

<Card title="通用">
  <Row label="开机自动启动" desc="登录后静默驻留托盘，不弹窗口">
    <Switch checked={s.autostart} label="开机自动启动" onchange={(v) => save({ autostart: v })} />
  </Row>
  <Row label="外观">
    <Segmented value={s.theme} options={THEMES} onchange={(v) => save({ theme: v })} />
  </Row>
</Card>

<Card title="关于">
  <Row label={`VoiceKey ${info.version}`} desc="Windows · 手机当麦克风，识别在电脑上完成" />
</Card>

<style>
  .sub { margin: 4px 0 16px; color: var(--fg-2); }
  :global(.card) { margin-bottom: 12px; }
</style>
