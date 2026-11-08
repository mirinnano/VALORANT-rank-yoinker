// 録画から切り出したクリップの一覧と再生。
const clipState = { playing: null, stopAt: null };

function formatClipLength(ms) {
  const seconds = Math.round(ms / 1000);
  return seconds < 60 ? `${seconds}秒` : `${Math.floor(seconds / 60)}分${String(seconds % 60).padStart(2, "0")}秒`;
}

function formatBytes(bytes) {
  return bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(1)} GB` : `${Math.round(bytes / 1024 ** 2)} MB`;
}

const CLIP_ORDER = { clutch: 0, kills: 1, defuse: 2, manual: 3 };

function clipPlayer() {
  const wrap = element("section", "clip-player");
  const video = document.createElement("video");
  video.id = "clip-video";
  video.controls = true;
  video.preload = "metadata";
  video.addEventListener("timeupdate", () => {
    if (clipState.stopAt !== null && video.currentTime >= clipState.stopAt) {
      video.pause();
      clipState.stopAt = null;
    }
  });
  const caption = element("p", "clip-caption", "クリップを選ぶと、ここで再生します。");
  caption.id = "clip-caption";
  wrap.append(video, caption);
  return wrap;
}

function playClip(recording, clip) {
  const video = document.getElementById("clip-video");
  const source = window.__TAURI__.core.convertFileSrc(recording.path);
  const start = clip.start_ms / 1000;
  const begin = () => {
    video.currentTime = start;
    clipState.stopAt = clip.end_ms / 1000;
    video.play().catch(() => {});
  };
  document.getElementById("clip-caption").textContent =
    `${recording.map ?? "試合"}　${clip.round ? `第${clip.round}ラウンド　` : ""}${clip.title}`;
  if (clipState.playing !== recording.path) {
    clipState.playing = recording.path;
    video.src = source;
    video.addEventListener("loadedmetadata", begin, { once: true });
  } else {
    begin();
  }
}

function clipRow(recording, clip) {
  const item = element("li", `clip is-${clip.kind}`);
  const play = element("button", "clip-play", clip.title);
  play.type = "button";
  play.disabled = !recording.exists;
  play.addEventListener("click", () => playClip(recording, clip));
  const where = clip.round ? `第${clip.round}ラウンド` : "保存した場面";
  const meta = element("span", "clip-meta", `${where}　${formatClipLength(clip.end_ms - clip.start_ms)}`);
  const exportButton = element("button", "tag", clip.exported_path ? "書き出し済み" : "MP4 に書き出す");
  exportButton.type = "button";
  exportButton.disabled = !recording.exists;
  if (clip.exported_path) exportButton.title = clip.exported_path;
  exportButton.addEventListener("click", async () => {
    exportButton.disabled = true;
    exportButton.textContent = "書き出しています…";
    try {
      const path = await invoke("export_clip", { clipId: clip.id });
      exportButton.textContent = "書き出し済み";
      exportButton.title = path;
      meta.textContent = `${where}　保存先: ${path}`;
    } catch (error) {
      exportButton.textContent = "書き出せませんでした";
      meta.textContent = String(error);
    } finally {
      exportButton.disabled = false;
    }
  });
  const share = element("button", "primary clip-share", "共有用に書き出す");
  share.type = "button";
  share.disabled = !recording.exists;
  share.title = "冒頭のカードと選手プレート、キルフィードを重ねた動画を作ります";
  share.addEventListener("click", () => exportShareClip(recording, clip, share, meta));
  item.append(play, meta, exportButton, share);
  return item;
}

function recordingBlock(recording) {
  const block = element("section", "clip-recording");
  const date = new Intl.DateTimeFormat("ja-JP", { month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit" })
    .format(new Date(recording.started_ms));
  const head = element("div", "clip-head");
  head.append(
    element("h2", "", `${recording.map ?? "試合"} / ${recording.mode ?? ""}`.replace(/ \/ $/, "")),
    element("span", "clip-meta", `${date}　${formatClipLength(recording.length_ms)}　${formatBytes(recording.bytes)}`),
  );

  // Riot 側と手元の時計のずれを直す。場面が早すぎる、遅すぎるときに使う。
  const offset = element("div", "clip-offset");
  const label = element("span", "clip-meta", `時刻の補正 ${(recording.offset_ms / 1000).toFixed(1)}秒`);
  const nudge = (delta) => async () => {
    label.textContent = "作り直しています…";
    try {
      await invoke("set_recording_offset", { id: recording.id, offsetMs: recording.offset_ms + delta });
      await showClips(views.clips);
    } catch (error) {
      label.textContent = String(error);
    }
  };
  for (const [text, delta, hint] of [["−1秒", -1000, "場面を早める"], ["+1秒", 1000, "場面を遅らせる"]]) {
    const button = element("button", "tag", text);
    button.type = "button";
    button.title = hint;
    button.disabled = !recording.match_id;
    button.addEventListener("click", nudge(delta));
    offset.append(button);
  }
  offset.prepend(label);

  const remove = element("button", "tag", "録画を消す");
  remove.type = "button";
  remove.addEventListener("click", async () => {
    if (!window.confirm("この試合の録画を消します。書き出したクリップは残ります。元に戻せません。")) return;
    await invoke("delete_recording", { id: recording.id });
    await showClips(views.clips);
  });
  head.append(offset, remove);

  const clips = [...recording.clips].sort((a, b) => (CLIP_ORDER[a.kind] - CLIP_ORDER[b.kind]) || (b.kills - a.kills) || (a.start_ms - b.start_ms));
  block.append(head);
  if (!recording.exists) {
    block.append(noticeBox("録画ファイルが見つかりません", "ファイルを移動または削除した可能性があります。"));
  } else if (clips.length === 0) {
    block.append(noticeBox("クリップはまだありません", recording.match_id ? "試合の結果が届くと、自動で切り出します。" : "録画中に場面を残すキーを押すと、ここに並びます。"));
  } else {
    const list = element("ol", "clip-list");
    list.append(...clips.map((clip) => clipRow(recording, clip)));
    block.append(list);
  }
  return block;
}

async function showClips(view) {
  const [recordings, active] = await Promise.all([invoke("list_recordings"), invoke("recording_active")]);
  const nodes = [];
  if (active) nodes.push(noticeBox("録画中", "試合が終わると保存し、見せ場を切り出します。"));
  if (recordings.length === 0) {
    nodes.push(noticeBox("録画はまだありません", "設定の Recording で録画をオンにすると、次の試合から録画します。"));
    view.replaceChildren(...nodes);
    return;
  }
  const nameLabel = element("label", "clip-name-toggle");
  const nameBox = document.createElement("input");
  nameBox.type = "checkbox";
  nameBox.checked = readShareName();
  nameBox.addEventListener("change", () => writeShareName(nameBox.checked));
  nameLabel.append(nameBox, "共有用の動画に自分の名前を入れる");
  view.replaceChildren(...nodes, clipPlayer(), nameLabel, ...recordings.map(recordingBlock));
}

window.__TAURI__.event.listen("clips-updated", () => {
  if (!views.clips.hidden) showClips(views.clips);
});
window.__TAURI__.event.listen("clip-marked", ({ payload }) => {
  const badge = document.getElementById("live-badge");
  if (badge) {
    badge.hidden = false;
    badge.textContent = `場面を保存（${payload}）`;
  }
});
