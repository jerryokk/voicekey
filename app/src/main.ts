import { mount } from "svelte";
import "./lib/style.css";
import Settings from "./Settings.svelte";
import Hud from "./Hud.svelte";

const view: string = (window as any).__VK_VIEW ?? location.hash.slice(1);
const target = document.getElementById("app")!;
document.body.classList.add(view === "hud" ? "transparent" : "opaque");
mount(view === "hud" ? Hud : Settings, { target });
