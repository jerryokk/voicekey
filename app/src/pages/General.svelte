<script lang="ts">
  import { app, save } from "../lib/store.svelte";

  const s = $derived(app.s!);
  const info = $derived(app.info!);
  const THEMES: [string, string][] = [["system", "跟随系统"], ["light", "浅色"], ["dark", "深色"]];
</script>

<h2>通用</h2>
<div class="sub">启动与外观。</div>
<div class="group">
  <div class="row">
    <div class="lbl">开机自动启动</div>
    <button class="sw" class:on={s.autostart} onclick={() => save({ autostart: !s.autostart })} aria-label="开机自动启动"></button>
  </div>
  <div class="row">
    <div class="lbl">界面外观</div>
    <div class="segc">
      {#each THEMES as [k, v]}<button class:on={s.theme === k} onclick={() => save({ theme: k })}>{v}</button>{/each}
    </div>
  </div>
</div>
<h3>关于</h3>
<div class="group">
  <div class="row">
    <div class="lbl">VoiceKey {info.version}<small>Windows</small></div>
  </div>
</div>
