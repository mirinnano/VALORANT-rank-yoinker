// 常駐と更新の設定。値は Rust 側のファイルに保存する。
(() => {
  const invoke = window.__TAURI__.core.invoke;
  const listen = window.__TAURI__.event.listen;
  const boxes = {
    autostart: document.getElementById("desktop-autostart"),
    front: document.getElementById("desktop-front"),
    tray: document.getElementById("desktop-tray"),
    updates: document.getElementById("desktop-updates"),
  };
  const status = document.getElementById("desktop-status");
  const recording = {
    enabled: document.getElementById("recording-enabled"),
    audio: document.getElementById("recording-audio"),
    fps: document.getElementById("recording-fps"),
    bitrate: document.getElementById("recording-bitrate"),
    keep: document.getElementById("recording-keep"),
    max: document.getElementById("recording-max"),
    hotkey: document.getElementById("clip-hotkey"),
  };
  const recordingStatus = document.getElementById("recording-status");
  const banner = document.getElementById("update-banner");

  function showUpdate(info) {
    document.getElementById("update-text").textContent = `新しい版 ${info.version} があります。`;
    banner.hidden = false;
  }

  function saveSettings() {
    invoke("set_desktop_settings", {
      settings: {
        close_to_tray: boxes.tray.checked,
        bring_to_front: boxes.front.checked,
        check_updates: boxes.updates.checked,
        recording: {
          enabled: recording.enabled.checked,
          audio: recording.audio.checked,
          fps: Number(recording.fps.value),
          bitrate_mbps: Number(recording.bitrate.value),
          keep: Number(recording.keep.value),
          max_gb: Number(recording.max.value),
        },
        clip_hotkey: recording.hotkey.value,
      },
    })
      .then(() => { recordingStatus.textContent = ""; })
      .catch((error) => { recordingStatus.textContent = `設定を保存できませんでした: ${error}`; });
  }

  invoke("desktop_settings").then((view) => {
    boxes.autostart.checked = view.autostart;
    boxes.front.checked = view.bring_to_front;
    boxes.tray.checked = view.close_to_tray;
    boxes.updates.checked = view.check_updates;
    recording.enabled.checked = view.recording.enabled;
    recording.audio.checked = view.recording.audio;
    recording.fps.value = String(view.recording.fps);
    recording.bitrate.value = String(view.recording.bitrate_mbps);
    recording.keep.value = String(view.recording.keep);
    recording.max.value = String(view.recording.max_gb);
    recording.hotkey.value = view.clip_hotkey;
    status.textContent = `バージョン ${view.version}`;
  });

  boxes.autostart.addEventListener("change", async () => {
    try {
      boxes.autostart.checked = await invoke("set_autostart", { enabled: boxes.autostart.checked });
    } catch (error) {
      boxes.autostart.checked = !boxes.autostart.checked;
      status.textContent = `自動起動を切り替えられませんでした: ${error}`;
    }
  });
  for (const box of [boxes.front, boxes.tray, boxes.updates, ...Object.values(recording)]) {
    box.addEventListener("change", saveSettings);
  }

  document.getElementById("check-update").addEventListener("click", async () => {
    status.textContent = "確かめています…";
    try {
      const info = await invoke("check_update");
      if (info) showUpdate(info);
      status.textContent = info ? `新しい版 ${info.version} があります` : "最新の版です";
    } catch (error) {
      status.textContent = `確かめられませんでした: ${error}`;
    }
  });

  document.getElementById("update-install").addEventListener("click", async (event) => {
    event.target.disabled = true;
    document.getElementById("update-text").textContent = "ダウンロードしています。終わると再起動します…";
    try {
      await invoke("install_update");
    } catch (error) {
      event.target.disabled = false;
      document.getElementById("update-text").textContent = `更新できませんでした: ${error}`;
    }
  });
  document.getElementById("update-dismiss").addEventListener("click", () => { banner.hidden = true; });

  listen("update-available", (event) => showUpdate(event.payload));
})();
