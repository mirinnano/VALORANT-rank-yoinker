// タイトルバーの代わりに、ヘッダーのボタンでウィンドウを操作する。
const appWindow = window.__TAURI__.window.getCurrentWindow();

document.getElementById("window-minimize").addEventListener("click", () => appWindow.minimize());
document.getElementById("window-maximize").addEventListener("click", () => appWindow.toggleMaximize());
document.getElementById("window-close").addEventListener("click", () => appWindow.close());
