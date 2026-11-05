import { mount } from "svelte";
import "./lib/style.css";
import Settings from "./Settings.svelte";
import Hud from "./Hud.svelte";
import Qr from "./Qr.svelte";
import Inbox from "./Inbox.svelte";

// 窗口都由后端建，建窗时注入 __VK_VIEW 指明自己是哪一个（见 ui/windows.rs）
const view: string = (window as any).__VK_VIEW;
const target = document.getElementById("app")!;
if (view === "hud") document.body.classList.add("transparent");
const VIEWS = { hud: Hud, qr: Qr, inbox: Inbox } as const;
mount(VIEWS[view as keyof typeof VIEWS] ?? Settings, { target });
