import { invoke } from "@tauri-apps/api/core";
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

export interface Settings {
  channel: Channel;
  streaming: boolean;
  liveText: boolean;
  multi: Channel[];
  lastPick: Channel | null;
  pickCounts: Partial<Record<Channel, number>>;
  autostart: boolean;
  theme: string;
}

export interface OfflineStatus { state: "missing" | "downloading" | "ready" | "failed"; progress?: number; error?: string }
export interface UpdateStatus { state: "idle" | "checking" | "latest" | "available" | "downloading" | "error"; version?: string; progress?: number; error?: string }

/** 手机接入：地址要等隧道就绪才有，二维码由后端生成成 SVG */
export interface Remote { url: string | null; qr: string | null; state?: "starting" | "ready" | "failed"; error?: string | null; lanOnly?: boolean }

export interface AppInfo {
  settings: Settings; platform: "win"; version: string; channels: Channel[];
  models: Partial<Record<Channel, OfflineStatus>>;
  paused: boolean; remote: Remote; update: UpdateStatus;
}

export const call = invoke;
export const on = <T>(name: string, f: (p: T) => void): Promise<UnlistenFn> => listen<T>(name, (e) => f(e.payload));

export const CH: Record<Channel, { name: string; short: string; desc: string; tags: string[]; logo: string }> = {
  doubao: { name: "豆包输入法", short: "豆包", desc: "响应快、中英混说准确，适合日常输入。", tags: ["在线", "流式", "推荐"], logo: doubao },
  wetype: { name: "微信输入法", short: "微信", desc: "口语化表达识别稳定，数字自动规整。", tags: ["在线", "流式"], logo: wetype },
  qwen: { name: "千问输入法", short: "千问", desc: "千问输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: qwen },
  baidu: { name: "百度输入法", short: "百度", desc: "百度输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: baidu },
  sogou: { name: "搜狗输入法", short: "搜狗", desc: "搜狗输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: sogou },
  iflytek: { name: "讯飞输入法", short: "讯飞", desc: "讯飞输入法官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: iflytek },
  chatterfly: { name: "Chatterfly", short: "Chatterfly", desc: "Chatterfly 官方云端识别，中文流式出字。", tags: ["在线", "流式"], logo: chatterfly },
  wetypeoffline: { name: "微信离线", short: "微信离线", desc: "微信输入法官方离线模型，本机运行，断网可用，不上传音频。", tags: ["离线", "约 100MB"], logo: wetype },
  all: { name: "多渠道", short: "多渠道", desc: "勾选的渠道同时识别、实时出字，说完在手机上挑一条上屏。", tags: ["并行", "流式候选"], logo: "" },
};

/** 需要下载模型的本地渠道及下载体积 */
export const LOCAL: Partial<Record<Channel, string>> = { wetypeoffline: "约 100MB" };

/** 根据平台与主题设置 html class */
export function applyLook(platform: string, theme: string) {
  const root = document.documentElement;
  root.classList.toggle("win", platform === "win");
  const dark = theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  root.classList.toggle("dark", dark);
}
