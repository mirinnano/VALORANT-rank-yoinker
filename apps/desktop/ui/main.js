const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const views = {
  lobby: document.getElementById("lobby-view"),
  shop: document.getElementById("shop-view"),
  home: document.getElementById("home-view"),
  match: document.getElementById("match-view"),
  review: document.getElementById("review-view"),
  stats: document.getElementById("stats-view"),
  settings: document.getElementById("settings-view"),
  history: document.getElementById("history-view"),
  clips: document.getElementById("clips-view"),
};
const IMAGE_RETRIES = 3;
const IMAGE_RETRY_MS = 30_000;
const app = { lastEvent: null, state: null, profile: null, profileSignature: "", queueTimer: null };

function noticeBox(title, detail) {
  const box = element("p", "notice");
  box.append(element("strong", "", title), detail);
  return box;
}

function setHeading(title, subtitle, score = null) {
  document.getElementById("title").textContent = title;
  const line = document.getElementById("subtitle");
  line.textContent = subtitle;
  if (!score) {
    app.score = null;
    return;
  }
  const value = element("span", "score-value", `${score[0]} - ${score[1]}`);
  line.append(subtitle ? "、" : "", value);
  if (app.score && app.score !== value.textContent) riseIn(value);
  app.score = value.textContent;
}

const LOBBY_LABELS = { queueing: "キュー中", match_found: "マッチ成立", custom: "カスタムゲーム準備中" };

function formatClock(ms) {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** ロビーの状態を見出しの1行にする。 */
function lobbySubtitle(snapshot, now = Date.now()) {
  const lobby = snapshot.lobby;
  if (!lobby) return snapshot.mode ?? "";
  const status = lobby.phase === "ready" ? (lobby.idle ? "離席中" : null) : LOBBY_LABELS[lobby.phase];
  const clock = lobby.phase === "queueing" && lobby.queue_started_ms
    ? ` ${formatClock(now - lobby.queue_started_ms)}`
    : "";
  const limit = lobby.max_party_size ? `/${lobby.max_party_size}` : "人";
  const party = lobby.party_size > 1 || lobby.open
    ? `${lobby.open ? "公開パーティ" : "パーティ"} ${lobby.party_size}${limit}`
    : null;
  const mode = lobby.phase === "custom" ? null : snapshot.mode;
  return [status && `${status}${clock}`, mode, party].filter(Boolean).join("、");
}

/** キュー中だけ、見出しの経過時間を毎秒進める。 */
function syncQueueClock(event) {
  const ticking = event.lobby?.phase === "queueing" && Boolean(event.lobby.queue_started_ms);
  if (ticking && app.queueTimer === null) {
    app.queueTimer = setInterval(() => {
      document.getElementById("subtitle").textContent = lobbySubtitle(app.lastEvent);
      updateLobbyClock(app.lastEvent);
    }, 1000);
  } else if (!ticking && app.queueTimer !== null) {
    clearInterval(app.queueTimer);
    app.queueTimer = null;
  }
}

/** WebSocket で動いているかを、常に見えるようにする。 */
function updateLiveBadge(event) {
  const badge = document.getElementById("live-badge");
  badge.hidden = event.kind !== "snapshot";
  badge.classList.toggle("is-live", Boolean(event.realtime));
  badge.textContent = event.realtime ? "リアルタイム" : "3秒ごとに更新";
}

function showView(name) {
  for (const [key, view] of Object.entries(views)) view.hidden = key !== name;
  for (const button of document.querySelectorAll("nav button")) {
    button.setAttribute("aria-pressed", String(button.dataset.view === name));
  }
  if (name === "shop") showShop(views.shop);
  if (name === "clips") showClips(views.clips);
  if (name === "review") {
    showReviewList().catch((error) => {
      views.review.replaceChildren(noticeBox("振り返りを開けませんでした", String(error)));
    });
  }
  if (name === "history") {
    invoke("recent_encounters")
      .then((entries) => renderHistory(views.history, entries))
      .catch((error) => {
        views.history.replaceChildren(noticeBox("遭遇履歴を読み込めませんでした", String(error)));
      });
  }
}

function renderSnapshot(snapshot) {
  const label = STATE_LABELS[snapshot.state];
  const inMatch = snapshot.state !== "menus";
  const name = snapshot.profile?.name ?? app.profile?.name;
  setHeading(
    inMatch ? snapshot.map ?? label : name ?? "ホーム",
    inMatch ? [label, snapshot.mode].filter(Boolean).join("、") : lobbySubtitle(snapshot),
    snapshot.state === "in_game" ? snapshot.score : null,
  );

  if (snapshot.state !== app.state) {
    // 試合の開始と終了に合わせて、見るべき画面へ移る。
    // 試合の途中で起動した場合も、最初の状態から対戦へ移る。
    if (inMatch && (app.state === "menus" || app.state === null)) {
      if (app.state === "menus") announceMatch(snapshot);
      showView("match");
      enterView(views.match);
    }
    const matchEnded = !inMatch && app.state === "in_game";
    app.state = snapshot.state;
    if (matchEnded) {
      if (snapshot.profile) app.profile = snapshot.profile;
      reviewState.review = null;
      reviewState.opening = null;
      reviewState.justFinished = true;
      afterMatch.matchId = snapshot.finished_match ?? null;
      homeReveal.pending = true;
      showView("review");
      if (snapshot.finished_match) openReview(snapshot.finished_match);
    }
  }

  if (snapshot.profile) app.profile = snapshot.profile;
  const signature = JSON.stringify(app.profile);
  if (signature !== app.profileSignature) {
    app.profileSignature = signature;
    renderHome(views.home, app.profile);
    renderStats(views.stats, app.profile?.summary);
    refreshAfterMatch();
  }
  if (!app.profile) renderHome(views.home, null);
  if (!inMatch) renderLobby(views.lobby, snapshot, app.profile);
  checkShopInBackground();
  renderMatch(views.match, snapshot);
}

function renderEvent(event) {
  app.lastEvent = event;
  syncQueueClock(event);
  updateLiveBadge(event);
  if (event.kind === "snapshot") {
    renderSnapshot(event);
    return;
  }
  app.state = null;
  app.profile = null;
  app.profileSignature = "";
  setHeading("VALORANT を待っています", "");
  const waiting = () => {
    const box = noticeBox("VALORANT を起動してください", event.reason ?? "起動すると自動で接続します。");
    box.prepend(element("span", "waiting-dot"));
    return box;
  };
  lobbyView.signature = "";
  views.lobby.replaceChildren(waiting());
  views.home.replaceChildren(waiting());
  views.match.classList.add("is-empty");
  views.match.replaceChildren(waiting());
}

for (const button of document.querySelectorAll("nav button")) {
  button.addEventListener("click", () => showView(button.dataset.view));
}
/** 画像は後から届くので、届いたら描き直す。失敗しても画像なしで動く。 */
async function loadImages(attempt = 0) {
  try {
    Object.assign(images, await invoke("image_catalog"));
    app.profileSignature = "";
    lobbyView.signature = "";
    match.signature = "";
    match.marks.forEach((mark) => { delete mark.dataset.agent; });
    match.heroMap = undefined;
    if (app.lastEvent) renderEvent(app.lastEvent);
  } catch {
    if (attempt < IMAGE_RETRIES) setTimeout(() => loadImages(attempt + 1), IMAGE_RETRY_MS);
  }
}

listen("app-event", ({ payload }) => renderEvent(payload));

invoke("latest_event").then(renderEvent);
loadImages();
