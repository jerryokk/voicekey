import { mount } from "svelte";
import "./lib/style.css";
import Settings from "./Settings.svelte";
import Hud from "./Hud.svelte";
import Qr from "./Qr.svelte";

// 三个窗口都由后端建，建窗时注入 __VK_VIEW 指明自己是哪一个（见 ui/windows.rs）
const view: string = (window as any).__VK_VIEW;
const target = document.getElementById("app")!;
if (view === "hud") document.body.classList.add("transparent");
mount(view === "hud" ? Hud : view === "qr" ? Qr : Settings, { target });
