<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, on, applyTheme, CH, type Channel } from "./lib/api";
  import ChIcon from "./lib/ChIcon.svelte";

  type Row = { channel: Channel; name: string; text: string; state: "listen" | "wait" | "final" | "error" | "skip" };
  type State = { state: string; text: string; channel: Channel; rows: { rows: Row[]; sel: number } | null };

  let st = $state<State>({ state: "hidden", text: "", channel: "doubao", rows: null });
  /** 电脑本机说话时的电平：用户手里没有手机看波形，这是确认「电脑真的在收音」的唯一反馈 */
  let level = $state(0);
  let local = $state(false);
  let shaking = $state(-1);

  onMount(() => {
    // 胶囊永远是深色：它浮在任意窗口上，深底白字在浅色和深色背景上都读得清
    applyTheme("dark");
    const un = [
      on<Partial<State>>("hud", (p) => {
        st = { ...st, text: "", rows: null, ...p };
        if (st.state !== "listen") level = 0;
        if (st.state === "hidden") local = false;
      }),
      on<number>("level", (v) => ((level = v), (local = true))),
      on<number>("shake", (i) => {
        shaking = -1;
        requestAnimationFrame(() => (shaking = i));
      }),
    ];
    return () => un.forEach((p) => p.then((f) => f()));
  });

  const picking = $derived(st.state === "pick");
  const BARS = 5;
  const SHAPE = [0.55, 0.85, 1, 0.8, 0.6];
</script>

<div class="wrap" class:rows={!!st.rows}>
  {#if st.rows}
    <div class="panel" class:show={st.state !== "hidden"} role="listbox" aria-label="选一条上屏">
      <div class="head">
        {#if picking}<b>选一条上屏</b><span>点一下即打进输入框 · Esc 放弃</span>
        {:else}<span class="rec"></span><b>正在听</b><span>松开后挑一条</span>{/if}
      </div>
      {#each st.rows.rows as r, i (r.channel)}
        <button
          class="row {r.state}"
          class:sel={i === st.rows.sel}
          class:shake={shaking === i}
          role="option"
          aria-selected={i === st.rows.sel}
          disabled={!picking}
          onclick={() => invoke("hud_choose", { row: i })}
          onanimationend={() => (shaking = -1)}
        >
          <ChIcon ch={r.channel} size="xs" />
          <span class="name">{CH[r.channel].short}</span>
          <span class="txt">{r.text || " "}</span>
          <span class="st"></span>
        </button>
      {/each}
    </div>
  {:else}
    <div class="capsule" class:show={st.state !== "hidden"} class:error={st.state === "error"}>
      {#if st.state === "error"}
        <span class="badge">!</span>
      {:else}
        <ChIcon ch={st.channel} size="sm" />
      {/if}
      {#if st.state === "listen"}
        {#if local}
          <span class="meter" aria-label="电脑麦克风收音中">
            {#each SHAPE as k}<i style="height:{4 + level * 14 * k}px"></i>{/each}
          </span>
        {:else}
          <span class="rec"></span>
        {/if}
      {/if}
      {#if st.text}
        <div class="live"><span>{st.text}</span></div>
      {:else if st.state === "wait"}
        <div class="hint">识别中…</div>
      {/if}
      {#if st.state === "wait"}<span class="spin"></span>{/if}
    </div>
  {/if}
</div>

<style>
  .wrap { height: 100vh; display: flex; align-items: flex-end; justify-content: center; padding-bottom: 14px; }
  .capsule {
    display: flex; align-items: center; gap: 12px; height: 46px; max-width: 640px; padding: 0 18px 0 12px; border-radius: 23px;
    background: rgba(18, 19, 30, 0.86); color: #fff; box-shadow: 0 8px 28px rgba(0, 0, 0, 0.35), inset 0 0 0 1px rgba(255, 255, 255, 0.08);
    opacity: 0; transform: translateY(8px) scale(0.98); transition: opacity 0.16s, transform 0.16s;
  }
  .capsule.show, .panel.show { opacity: 1; transform: none; }
  /* 出错要和录音看得出区别：底色偏红，不再只是中间多了个感叹号 */
  .capsule.error {
    background: rgba(58, 20, 16, 0.9);
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.35), inset 0 0 0 1px rgba(255, 120, 90, 0.28);
  }
  .rec { width: 9px; height: 9px; border-radius: 50%; background: #d9552b; flex: none; animation: pulse 1s ease-in-out infinite; }
  .meter { display: flex; align-items: center; gap: 3px; height: 20px; flex: none; }
  .meter i { width: 3px; border-radius: 2px; background: #e8734a; transition: height 0.06s linear; }
  .live { min-width: 0; max-width: 460px; white-space: nowrap; overflow: hidden; direction: rtl; text-align: left; font-size: 14px; }
  .live span { direction: ltr; unicode-bidi: plaintext; }
  .hint { font-size: 13px; opacity: 0.7; }
  .badge {
    width: 22px; height: 22px; flex: none; border-radius: 6px;
    display: grid; place-items: center;
    background: #c0392b; color: #fff; font-size: 13px; font-weight: 600; line-height: 1;
  }
  .spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid rgba(255, 255, 255, 0.25); border-top-color: #fff; animation: spin 0.8s linear infinite; flex: none; }

  /* 多渠道候选（电脑本机说话时） */
  .panel {
    width: 600px; max-height: 390px; overflow-y: auto; display: flex; flex-direction: column; gap: 5px;
    padding: 10px; border-radius: 18px; color: #fff;
    background: rgba(18, 19, 30, 0.92); box-shadow: 0 10px 32px rgba(0, 0, 0, 0.4), inset 0 0 0 1px rgba(255, 255, 255, 0.08);
    opacity: 0; transform: translateY(8px) scale(0.98); transition: opacity 0.16s, transform 0.16s;
  }
  .head { display: flex; align-items: center; gap: 8px; padding: 2px 6px 6px; font-size: 13px; }
  .head b { font-weight: 600; }
  .head span:last-child { margin-left: auto; font-size: 12px; opacity: 0.55; }
  .row {
    display: flex; align-items: center; gap: 10px; padding: 9px 12px; border-radius: 11px; text-align: left;
    border: 1px solid transparent; background: rgba(255, 255, 255, 0.05); color: inherit; font: inherit; cursor: pointer;
  }
  .row:disabled { cursor: default; }
  .row:not(:disabled):hover { background: rgba(255, 255, 255, 0.1); }
  .row.sel { border-color: rgba(255, 255, 255, 0.5); background: rgba(255, 255, 255, 0.1); }
  .row .name { flex: none; width: 60px; font-size: 12.5px; font-weight: 600; }
  .row .txt { flex: 1; min-width: 0; font-size: 13.5px; opacity: 0.85; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row .st { flex: none; width: 7px; height: 7px; border-radius: 50%; background: rgba(255, 255, 255, 0.3); }
  .row.listen .st, .row.wait .st { background: #e8734a; animation: pulse 1s ease-in-out infinite; }
  .row.final .st { background: #3fb98a; }
  .row.error .st, .row.skip .st { background: #e2685a; }
  .row.error .txt, .row.skip .txt { opacity: 0.5; }
  .row.shake { animation: shake 0.4s ease; }
  @keyframes shake { 20%, 60% { transform: translateX(-5px); } 40%, 80% { transform: translateX(5px); } }
</style>
