import { invoke, on, applyTheme, type AppInfo, type Settings, type SettingsPatch, type OfflineStatus, type Channel, type Remote, type Last } from "./api";

export const app = $state({
  info: null as AppInfo | null,
  s: null as Settings | null,
  models: {} as Partial<Record<Channel, OfflineStatus>>,
  remote: { url: null, qr: null, phase: "starting", error: null, devices: 0 } as Remote,
  last: null as Last | null,
  /** 最近一次命令失败的原因，几秒后自动消失 */
  error: null as string | null,
});

let errorTimer: ReturnType<typeof setTimeout> | undefined;

function fail(e: unknown) {
  app.error = String(e);
  clearTimeout(errorTimer);
  errorTimer = setTimeout(() => (app.error = null), 5000);
}

/** 先在本地生效让界面跟手，再把改了的字段交给后端；后端广播的 settings 会覆盖回权威值。
 *  失败时说出原因（比如自启动没设上），再从后端取一次设置：命令没走到广播那步，本地的乐观值也要撤回 */
export function save(patch: SettingsPatch) {
  if (!app.s) return;
  app.s = { ...app.s, ...patch };
  invoke("set_settings", { patch }).catch(async (e) => {
    fail(e);
    app.s = (await invoke<AppInfo>("get_state")).settings;
  });
}

/** 断开手机：轮换 token，旧二维码立即失效。界面靠随后的 remote 事件刷新 */
export const disconnect = () => invoke("disconnect_phone").catch(fail);

/** 隧道失败后整套重新拉起（公网地址会换一个）。状态随 remote 事件变化 */
export const retry = () => invoke("retry_remote");

/** 手机接入状态的指示点与一句话描述：侧栏、连接页、扫码面板共用这一套判断与措辞 */
export function link(r: Remote) {
  if (r.phase === "failed") return { dot: "err" as const, text: "公网通道没有建立" };
  if (r.phase !== "ready" || !r.qr) return { dot: "busy" as const, text: "正在申请公网地址" };
  return { dot: "ok" as const, text: r.devices > 0 ? `${r.devices} 台手机在线` : "可以扫码连接" };
}

/** remote 事件不带数据：二维码按当前 token 现画，详情统一从 get_state 拉 */
async function refreshRemote() {
  app.remote = (await invoke<AppInfo>("get_state")).remote;
}

export async function init() {
  // 先挂好监听再取状态：隧道可能在 get_state 返回前后就绪，晚挂会漏掉那次广播
  await Promise.all([
    on("remote", refreshRemote),
    on<Settings>("settings", (s) => {
      app.s = s;
      applyTheme(s.theme);
    }),
    on<{ ch: Channel; status: OfflineStatus }>("model", (v) => (app.models[v.ch] = v.status)),
    on<Last>("last", (v) => (app.last = v)),
  ]);
  const info = await invoke<AppInfo>("get_state");
  app.info = info;
  app.s = info.settings;
  app.models = info.models;
  app.remote = info.remote;
  app.last = info.last;
  applyTheme(info.settings.theme);
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => app.s && applyTheme(app.s.theme));
}
