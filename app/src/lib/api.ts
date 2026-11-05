export { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import doubao from "../assets/logos/doubao.png";
import wetype from "../assets/logos/wetype.png";
import qwen from "../assets/logos/qwen.png";
import baidu from "../assets/logos/baidu.png";
import sogou from "../assets/logos/sogou.png";
import iflytek from "../assets/logos/iflytek.png";
import chatterfly from "../assets/logos/chatterfly.png";

export type Channel =
  | "doubao" | "wetype" | "qwen" | "baidu" | "sogou" | "iflytek" | "chatterfly" | "wetypeoffline" | "all";

export type Theme = "system" | "light" | "dark";

export interface Settings {
  channel: Channel;
  streaming: boolean;
  liveText: boolean;
  multi: Channel[];
  lastPick: Channel | null;
  pickCounts: Partial<Record<Channel, number>>;
  autostart: boolean;
  theme: Theme;
  /** 触摸板速度倍率，范围见 AppInfo.pointerSpeed */
  pointerSpeed: number;
  /** 电脑本机麦克风 */
  local: LocalMic;
}

export type Hotkey = "rightAlt" | "rightCtrl";

export interface LocalMic {
  enabled: boolean;
  /** WASAPI 端点 id；null 跟随系统默认 */
  device: string | null;
  hotkey: Hotkey;
}

export interface OfflineStatus { state: "missing" | "downloading" | "ready" | "failed"; progress?: number; error?: string }

/** 手机接入：地址要等隧道就绪才有，二维码由后端生成成 SVG */
export interface Remote {
  url: string | null; qr: string | null;
  phase: "starting" | "ready" | "failed";
  error: string | null;
  /** 在线手机数 */
  devices: number;
}

export interface Last { text: string; channel: Channel; at: number }

/** 电脑本机麦克风的运行状态 */
export interface LocalStatus {
  state: "off" | "ready" | "error";
  error: string | null;
  /** 出错原因是 Windows 隐私设置 */
  privacy: boolean;
  /** 能用，但要让用户知道的事（所选设备不在，改用了系统默认） */
  note: string | null;
}

export interface Mic { id: string; name: string }

export interface InboxFile { name: string; path: string; size: number; thumb?: string; gone: boolean }

/** 手机发到电脑的一项 */
export type InboxCard = { id: number; at: number; open: boolean } & (
  | { kind: "clip"; text: string }
  | { kind: "text"; text: string }
  | { kind: "files"; files: InboxFile[]; of: number }
);

export interface AppInfo {
  settings: Settings; version: string; channels: Channel[];
  /** 触摸板速度的 [最小, 最大]，后端夹值用的同一份 */
  pointerSpeed: [number, number];
  models: Partial<Record<Channel, OfflineStatus>>;
  remote: Remote; last: Last | null;
  local: LocalStatus;
  inbox: InboxCard[];
}

/** 按住说话可选的键：只有右 Alt、右 Ctrl */
export const HOTKEYS: { v: Hotkey; label: string }[] = [
  { v: "rightAlt", label: "右 Alt" },
  { v: "rightCtrl", label: "右 Ctrl" },
];

export const hotkeyName = (k: Hotkey) => HOTKEYS.find((h) => h.v === k)?.label ?? k;

export function fileSize(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1 << 20) return `${Math.round(n / 1024)} KB`;
  if (n < 1 << 30) return `${(n / (1 << 20)).toFixed(1)} MB`;
  return `${(n / (1 << 30)).toFixed(2)} GB`;
}

/** 界面能改的设置。上屏历史由后端自己记，不从界面写回 */
export type SettingsPatch = Partial<Omit<Settings, "lastPick" | "pickCounts">>;

export const on = <T>(name: string, f: (p: T) => void): Promise<UnlistenFn> => listen<T>(name, (e) => f(e.payload));

const OFFLINE_SIZE = "约 100MB";

export const CH: Record<Channel, { name: string; short: string; desc: string; tags: string[]; logo: string }> = {
  doubao: { name: "豆包输入法", short: "豆包", desc: "响应快、中英混说准确，适合日常输入。", tags: ["在线", "流式", "推荐"], logo: doubao },
  wetype: { name: "微信输入法", short: "微信", desc: "口语化表达识别稳定，数字自动规整。", tags: ["在线", "流式"], logo: wetype },
  qwen: { name: "千问输入法", short: "千问", desc: "千问输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: qwen },
  baidu: { name: "百度输入法", short: "百度", desc: "百度输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: baidu },
  sogou: { name: "搜狗输入法", short: "搜狗", desc: "搜狗输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: sogou },
  iflytek: { name: "讯飞输入法", short: "讯飞", desc: "讯飞输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: iflytek },
  chatterfly: { name: "Chatterfly", short: "Chatterfly", desc: "Chatterfly 官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: chatterfly },
  wetypeoffline: { name: "微信离线", short: "微信离线", desc: "微信输入法官方离线模型，本机运行，断网可用，不上传音频。", tags: ["离线", OFFLINE_SIZE], logo: wetype },
  all: { name: "多渠道", short: "多渠道", desc: "勾选的渠道同时识别、实时出字，说完挑一条上屏。", tags: ["并行", "流式候选"], logo: "" },
};

/** 需要下载模型的本地渠道及下载体积 */
export const LOCAL: Partial<Record<Channel, string>> = { wetypeoffline: OFFLINE_SIZE };

/** 按主题切换 html 上的 dark class */
export function applyTheme(theme: Theme) {
  const dark = theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
}
