// 録画から切り出したクリップの一覧と再生。
const clipState = { playing: null, stopAt: null, selection: null, filter: "all" };
/** タイムラインで出来事を選んだときの、前後に含める時間。 */
const PICK_BEFORE_MS = 6_000;
const PICK_AFTER_MS = 3_000;
const NUDGE_MS = 1_000;

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
  const stage = element("div", "clip-stage");
  stage.append(video);
  wrap.append(stage, caption);
  return wrap;
}

/** 録画を `fromMs` から再生する。`stopMs` で止め、`kills`（自分のキル）の瞬間に演出を出す。 */
function playRange(recording, fromMs, stopMs, kills, caption) {
  const video = document.getElementById("clip-video");
  const begin = () => {
    video.currentTime = Math.max(fromMs, 0) / 1000;
    clipState.stopAt = stopMs === null ? null : stopMs / 1000;
    watchReactions(video, kills);
    video.play().catch(() => {});
  };
  document.getElementById("clip-caption").textContent = caption;
  if (clipState.playing !== recording.path) {
    clipState.playing = recording.path;
    video.src = window.__TAURI__.core.convertFileSrc(recording.path);
    video.addEventListener("loadedmetadata", begin, { once: true });
  } else {
    begin();
  }
  document.querySelector(".clip-player")?.scrollIntoView({ block: "nearest" });
}

function playClip(recording, clip) {
  const kills = clip.events
    .filter((event) => event.kind === "kill")
    .map((event) => ({ at_ms: clip.start_ms + event.at_ms, victim: event.victim, weapon: event.weapon, round: clip.round, headshot: event.headshot }));
  const caption = `${recording.map ?? "試合"}　${clip.round ? `第${clip.round}ラウンド　` : ""}${clip.title}`;
  playRange(recording, clip.start_ms, clip.end_ms, kills, caption);
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
  if (recording.exists && recording.match_id) {
    const timeline = element("section", "action-timeline");
    timeline.append(element("p", "clip-meta", "出来事を読み込んでいます…"));
    block.append(timeline);
    fillTimeline(timeline, recording);
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

// タイムライン。録画に映っている全員の出来事を並べ、選んだ範囲を書き出す。
const ACTION_SIDE_LABEL = { own: "あなた", ally: "味方", enemy: "相手" };
const ACTION_FILTERS = [["all", "全員"], ["own", "自分"], ["kill", "キルだけ"]];

function formatVideoTime(ms) {
  const total = Math.max(ms, 0) / 1000;
  const minutes = Math.floor(total / 60);
  return `${minutes}:${(total % 60).toFixed(1).padStart(4, "0")}`;
}

function actionText(action) {
  const who = action.actor ?? ACTION_SIDE_LABEL[action.actor_side] ?? "不明";
  if (action.kind === "plant") return `${who} が設置`;
  if (action.kind === "defuse") return `${who} が解除`;
  return `${who} → ${action.target ?? "不明"}`;
}

function visibleActions(actions) {
  if (clipState.filter === "own") {
    return actions.filter((action) => action.actor_side === "own" || action.target_side === "own");
  }
  if (clipState.filter === "kill") return actions.filter((action) => action.kind === "kill");
  return actions;
}

/** 範囲の中の自分のキル。演出に使う。 */
function ownKills(actions, from, to) {
  return actions
    .filter((action) => action.kind === "kill" && action.actor_side === "own" && action.at_ms >= from && action.at_ms <= to)
    .map((action) => ({ at_ms: action.at_ms, victim: action.target, weapon: action.weapon, round: action.round, headshot: action.headshot }));
}

function choose(recording, actions, action) {
  clipState.selection = {
    recordingId: recording.id,
    round: action.round,
    start: Math.max(action.at_ms - PICK_BEFORE_MS, 0),
    end: Math.min(action.at_ms + PICK_AFTER_MS, recording.length_ms),
  };
  renderSelection(recording, actions);
  const { start, end } = clipState.selection;
  playRange(recording, start, end, ownKills(actions, start, end), `第${action.round}ラウンド　${actionText(action)}`);
}

function actionStrip(recording, actions) {
  const strip = element("div", "action-strip");
  strip.setAttribute("role", "group");
  strip.setAttribute("aria-label", "録画の中の出来事の位置");
  const rounds = new Map();
  for (const action of actions) if (!rounds.has(action.round)) rounds.set(action.round, action.at_ms);
  for (const [round, at] of rounds) {
    const mark = element("span", "strip-round", `R${round}`);
    mark.style.left = `${(at / recording.length_ms) * 100}%`;
    strip.append(mark);
  }
  for (const action of visibleActions(actions)) {
    const tick = element("button", `strip-tick is-${action.actor_side ?? "none"} is-${action.kind}`);
    tick.type = "button";
    tick.style.left = `${(action.at_ms / recording.length_ms) * 100}%`;
    tick.title = `${formatVideoTime(action.at_ms)}　${actionText(action)}`;
    tick.setAttribute("aria-label", tick.title);
    tick.addEventListener("click", () => choose(recording, actions, action));
    strip.append(tick);
  }
  return strip;
}

function actionList(recording, actions) {
  const list = element("ol", "action-list");
  let round = null;
  for (const action of visibleActions(actions)) {
    if (action.round !== round) {
      round = action.round;
      list.append(element("li", "action-round", `ROUND ${round}`));
    }
    const item = element("li", `action is-${action.actor_side ?? "none"} is-${action.kind}`);
    const pick = element("button", "action-pick");
    pick.type = "button";
    pick.append(
      element("span", "action-time", formatVideoTime(action.at_ms)),
      action.actor ? agentIcon(action.actor, 24) : element("span", "action-dot"),
      element("span", "action-text", actionText(action)),
      element("span", "action-weapon", action.weapon ?? ""),
    );
    pick.addEventListener("click", () => choose(recording, actions, action));
    item.append(pick);
    list.append(item);
  }
  return list;
}

function renderTimeline(box, recording, actions) {
  const filters = element("div", "action-filters");
  for (const [key, label] of ACTION_FILTERS) {
    const button = element("button", "tag", label);
    button.type = "button";
    button.setAttribute("aria-pressed", String(clipState.filter === key));
    button.addEventListener("click", () => {
      clipState.filter = key;
      renderTimeline(box, recording, actions);
    });
    filters.append(button);
  }
  const selection = element("div", "selection");
  selection.id = `selection-${recording.id}`;
  box.replaceChildren(
    element("h3", "", "TIMELINE / 全員の出来事"),
    filters,
    actionStrip(recording, actions),
    selection,
    actionList(recording, actions),
  );
  if (clipState.selection?.recordingId === recording.id) renderSelection(recording, actions);
}

/** 選んだ範囲の調整と書き出し。 */
function renderSelection(recording, actions) {
  const box = document.getElementById(`selection-${recording.id}`);
  const chosen = clipState.selection;
  if (!box || !chosen || chosen.recordingId !== recording.id) return;
  const length = chosen.end - chosen.start;
  // 帯の上にも、選んだ範囲を示す。
  const strip = box.parentElement?.querySelector(".action-strip");
  if (strip) {
    const band = strip.querySelector(".strip-selection") ?? element("span", "strip-selection");
    band.style.left = `${(chosen.start / recording.length_ms) * 100}%`;
    band.style.width = `${(length / recording.length_ms) * 100}%`;
    strip.prepend(band);
  }
  const status = element("span", "clip-meta");
  status.setAttribute("role", "status");
  const nudge = (key, delta) => () => {
    const next = { ...chosen, [key]: Math.min(Math.max(chosen[key] + delta, 0), recording.length_ms) };
    if (next.end - next.start < NUDGE_MS) return;
    clipState.selection = next;
    renderSelection(recording, actions);
  };
  const button = (label, className, action) => {
    const node = element("button", className, label);
    node.type = "button";
    node.addEventListener("click", action);
    return node;
  };
  const ownKills = ownKills(actions, chosen.start, chosen.end);
  const range = element("div", "selection-range");
  range.append(
    element("b", "", `${formatVideoTime(chosen.start)} – ${formatVideoTime(chosen.end)}`),
    element("span", "clip-meta", `${formatClipLength(length)}　自分のキル ${ownKills.length}`),
  );
  const adjust = element("div", "selection-adjust");
  adjust.append(
    button("開始 −1秒", "tag", nudge("start", -NUDGE_MS)),
    button("開始 +1秒", "tag", nudge("start", NUDGE_MS)),
    button("終了 −1秒", "tag", nudge("end", -NUDGE_MS)),
    button("終了 +1秒", "tag", nudge("end", NUDGE_MS)),
    button("この範囲を再生", "tag", () => playRange(recording, chosen.start, chosen.end, ownKills, "選んだ範囲")),
  );
  const create = () =>
    invoke("create_clip", {
      recordingId: recording.id, round: chosen.round ?? null, startMs: chosen.start, endMs: chosen.end, title: "",
    });
  const plain = button("MP4 に書き出す", "tag", async () => {
    plain.disabled = true;
    status.textContent = "書き出しています…";
    try {
      const clip = await create();
      const path = await invoke("export_clip", { clipId: clip.id });
      status.textContent = `保存先: ${path}`;
    } catch (error) {
      status.textContent = `書き出せませんでした: ${error}`;
    } finally {
      plain.disabled = false;
    }
  });
  const share = button("共有用に書き出す", "primary", async () => {
    try {
      const clip = await create();
      await exportShareClip(recording, clip, share, status);
    } catch (error) {
      status.textContent = `書き出せませんでした: ${error}`;
    }
  });
  const actionsRow = element("div", "selection-actions");
  actionsRow.append(share, plain, status);
  box.replaceChildren(range, adjust, actionsRow);
}

async function fillTimeline(box, recording) {
  try {
    const actions = await invoke("recording_actions", { id: recording.id });
    if (actions.length === 0) {
      box.replaceChildren(element("p", "clip-meta", "試合の結果が届くと、全員の出来事がここに並びます。"));
      return;
    }
    renderTimeline(box, recording, actions);
  } catch (error) {
    box.replaceChildren(element("p", "clip-meta", `出来事を読み込めませんでした: ${error}`));
  }
}
