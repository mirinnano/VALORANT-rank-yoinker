const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const views = {
  home: document.getElementById("home-view"),
  match: document.getElementById("match-view"),
  review: document.getElementById("review-view"),
  settings: document.getElementById("settings-view"),
  history: document.getElementById("history-view"),
};
const IMAGE_RETRIES = 3;
const IMAGE_RETRY_MS = 30_000;
const app = { lastEvent: null, state: null, profile: null, profileSignature: "" };

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

function showView(name) {
  for (const [key, view] of Object.entries(views)) view.hidden = key !== name;
  for (const button of document.querySelectorAll("nav button")) {
    button.setAttribute("aria-pressed", String(button.dataset.view === name));
  }
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
    [inMatch ? label : null, snapshot.mode].filter(Boolean).join("、"),
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
  }
  if (!app.profile) renderHome(views.home, null);
  renderMatch(views.match, snapshot);
}

function renderEvent(event) {
  app.lastEvent = event;
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
