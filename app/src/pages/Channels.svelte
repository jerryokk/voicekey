<script lang="ts">
  import { app, save } from "../lib/store.svelte";
  import { call, CH, LOCAL, type Channel } from "../lib/api";
  import ChIcon from "../lib/ChIcon.svelte";
  import Card from "../lib/ui/Card.svelte";

  const s = $derived(app.s!);
  const engines = $derived(app.info!.channels.filter((c) => c !== "all"));
  const st = (c: Channel) => app.models[c] ?? { state: "missing" as const };
  const pick = (c: Channel) => save({ channel: c });
  const dl = (e: Event, c: Channel) => { e.stopPropagation(); call("model_download", { ch: c }); };

  const toggle = (c: Channel) => {
    const on = s.multi.includes(c);
    if (on && s.multi.length <= 2) return;
    save({ channel: "all", multi: engines.filter((e) => (e === c ? !on : s.multi.includes(e))) });
  };

  let advanced = $state(false);
</script>

<h1>渠道</h1>
<p class="sub">选择语音转文字的服务。默认那个要一直好用，其余作为备选。</p>

<div class="grid">
  {#each engines as c}
    {@const m = st(c)}
    {@const cur = s.channel === c}
    <button class="ch" class:on={cur} onclick={() => pick(c)}>
      <div class="top">
        <ChIcon ch={c} />
        <b>{CH[c].name}</b>
        {#if cur}<span class="tagcur">当前</span>{/if}
      </div>
      <p>{CH[c].desc}</p>
      <div class="tags">
        {#each CH[c].tags as t}<span class="pill">{t}</span>{/each}
        {#if LOCAL[c]}
          {#if m.state === "ready"}<span class="pill ok">模型就绪</span>
          {:else if m.state === "downloading"}<span class="pill">模型 {Math.round((m.progress ?? 0) * 100)}%</span>
          {:else if m.state === "failed"}<span class="pill err">模型失败</span>
          {/if}
        {/if}
      </div>
      {#if cur && LOCAL[c] && m.state !== "ready"}
        <div class="dl">
          {#if m.state === "downloading"}
            下载中 <span class="bar"><i style="width:{(m.progress ?? 0) * 100}%"></i></span>
          {:else}
            <span class="link" role="button" tabindex="-1" onclick={(e) => dl(e, c)} onkeydown={() => {}}>下载离线模型（{LOCAL[c]}）</span>
          {/if}
        </div>
      {/if}
    </button>
  {/each}
</div>

<div class="adv">
  <button class="btn ghost" onclick={() => (advanced = !advanced)}>
    {advanced ? "收起高级" : "高级：多渠道对比"}
  </button>
  {#if advanced}
    <Card sub="同时跑多个渠道，说完在手机上挑一条上屏">
      <div class="chips">
        {#each engines as c}
          {@const on = s.multi.includes(c)}
          <span class="chip" class:on class:lock={on && s.multi.length <= 2}
            role="checkbox" aria-checked={on} tabindex="-1"
            onclick={() => toggle(c)} onkeydown={() => {}}>
            <ChIcon ch={c} size="xs" />{CH[c].short}
          </span>
        {/each}
        <span class="cnt">已选 {s.multi.length} 个</span>
      </div>
      <p class="hint">启用后此页面会被当作「当前渠道」，手机上会出现候选列表。</p>
    </Card>
  {/if}
</div>

<style>
  .sub { margin: 4px 0 16px; color: var(--fg-2); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 10px; }

  .ch {
    display: flex; flex-direction: column; align-items: stretch; text-align: left;
    background: var(--card); border: 1px solid var(--line); border-radius: var(--r-lg);
    padding: 13px 14px; cursor: pointer; transition: border-color .12s, background .12s;
  }
  .ch:hover { border-color: var(--line-2); }
  .ch.on { border-color: var(--accent); background: var(--accent-soft); }
  .top { display: flex; align-items: center; gap: 9px; }
  .top b { font-size: 13px; font-weight: 500; }
  .tagcur {
    margin-left: auto; font-size: var(--fs-tag); color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-radius: var(--r-pill); padding: 1px 8px;
  }
  .ch p { margin: 7px 0 9px; font-size: var(--fs-tag); color: var(--fg-2); line-height: 1.6; }
  .tags { display: flex; flex-wrap: wrap; gap: 5px; }
  .pill {
    font-size: var(--fs-tag); color: var(--fg-3);
    border: 1px solid var(--line); border-radius: var(--r-pill); padding: 0 7px;
  }
  .pill.ok { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 35%, transparent); }
  .pill.err { color: var(--err); border-color: color-mix(in srgb, var(--err) 35%, transparent); }

  .dl { margin-top: 9px; display: flex; align-items: center; gap: 8px; font-size: var(--fs-tag); color: var(--fg-2); }
  .bar { flex: 1; height: 4px; border-radius: 2px; background: var(--line); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); }
  .link { color: var(--accent); cursor: pointer; }

  .adv { margin-top: 16px; display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; padding: 10px 0 4px; }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; padding: 3px 10px 3px 6px;
    border: 1px solid var(--line); border-radius: var(--r-pill);
    font-size: var(--fs-tag); color: var(--fg-2); cursor: pointer;
  }
  .chip.on { color: var(--fg); border-color: var(--accent); background: var(--accent-soft); }
  .chip.lock { cursor: default; }
  .cnt { margin-left: auto; font-size: var(--fs-tag); color: var(--fg-3); }
  .hint { margin: 0 0 12px; font-size: var(--fs-tag); color: var(--fg-3); }
</style>
