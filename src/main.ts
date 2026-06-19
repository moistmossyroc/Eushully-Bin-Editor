import App from "./App.svelte";
import { mount } from "svelte";

// Disable the native browser right-click context menu app-wide.
// Custom Svelte oncontextmenu handlers still fire on the target first and
// open their own menus; this listener just suppresses the default webview menu
// for everything that bubbles up unhandled.
window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
});

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
