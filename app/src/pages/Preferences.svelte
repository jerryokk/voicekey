<script lang="ts">
  import { app, save } from "../lib/store.svelte";
  import Card from "../lib/ui/Card.svelte";
  import Row from "../lib/ui/Row.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import type { Theme } from "../lib/api";

  const s = $derived(app.s!);
  const info = $derived(app.info!);
  const all = $derived(s.channel === "all");

  const THEMES: { v: Theme; label: string }[] = [
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

<Card title="触摸板">
  <Row label="光标速度" desc="1.0× 时慢慢划过整个触摸板，光标移动约半个屏幕；划得越快走得越远">
    <div class="speed">
      <input type="range" min={info.pointerSpeed[0]} max={info.pointerSpeed[1]} step="0.1" value={s.pointerSpeed} aria-label="光标速度"
        onchange={(e) => save({ pointerSpeed: Number(e.currentTarget.value) })} />
      <span>{s.pointerSpeed.toFixed(1)}×</span>
    </div>
  </Row>
</Card>

<Card title="通用">
  <Row label="开机自动启动" desc="登录后静默驻留托盘，不弹窗口">
    <Switch checked={s.autostart} label="开机自动启动" onchange={(v) => save({ autostart: v })} />
  </Row>
  <Row label="外观">
    <Segmented value={s.theme} options={THEMES} label="外观" onchange={(v) => save({ theme: v })} />
  </Row>
</Card>

<Card title="关于">
  <Row label={`VoiceKey ${info.version}`} desc="Windows · 按住说话，文字打进当前光标处" />
</Card>

<style>
  .speed { display: flex; align-items: center; gap: 10px; }
  .speed input { width: 150px; accent-color: var(--accent); }
  .speed span { width: 34px; text-align: right; font-size: var(--fs-sub); color: var(--fg-2); font-variant-numeric: tabular-nums; }
</style>
