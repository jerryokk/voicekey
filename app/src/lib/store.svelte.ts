import { call, on, applyLook, type AppInfo, type Settings, type OfflineStatus, type Channel, type Remote } from "./api";

export const app = $state({
  info: null as AppInfo | null,
  s: null as Settings | null,
  models: {} as Partial<Record<Channel, OfflineStatus>>,
  remote: { url: null, qr: null } as Remote,
  paused: false,
});

export function save(patch: Partial<Settings>) {
  if (!app.s) return;
  app.s = { ...app.s, ...patch };
  call("set_settings", { settings: app.s });
}

export function setPaused(on: boolean) {
  app.paused = on;
  call("set_paused", { on });
}

/** 隧道地址是异步就绪的，收到 remote 事件后再取一次 */
export async function refreshRemote() {
  const info = await call<AppInfo>("get_state");
  app.remote = info.remote;
}

export async function init() {
  // 先挂监听再取状态：隧道可能在 get_state 返回前后就绪，晚挂会漏掉那次广播
  on<{ state: string; error?: string }>("remote", (v) => {
    if (v.state === "ready" || v.state === "failed") refreshRemote();
  });
  const info = await call<AppInfo>("get_state");
  app.info = info;
  app.s = info.settings;
  app.models = info.models;
  app.remote = info.remote;
  app.paused = info.paused;
  const look = () => app.s && applyLook(info.platform, app.s.theme);
  look();
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", look);
  on<Settings>("settings", (s) => {
    app.s = s;
    look();
  });
  on<{ ch: Channel; status: OfflineStatus }>("model", (v) => (app.models[v.ch] = v.status));
}
