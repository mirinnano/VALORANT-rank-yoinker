const REVIEW_TAGS = ["立ち位置", "エイム", "判断", "連携", "経済", "アビリティ"];
const RETRY_DELAY_MS = 15_000;
const RETRY_LIMIT = 6;
const REVEAL_MS = 700;
const TONE_LABELS = { focus: "見直す", strength: "強み" };
const SCORE_COLUMNS = [
  ["", ""], ["ランク", ""], ["ACS", "num"], ["K", "num"], ["D", "num"], ["A", "num"],
  ["ADR", "num"], ["HS%", "num"], ["先手", "num"], ["被先手", "num"],
];

const reviewState = { requestId: 0, opening: null, justFinished: false, review: null, notes: new Map(), round: null, pointed: [], reviewed: new Set(), attempt: 0, timer: null };

function noteKey(round) {
  return round ?? "match";
}

function noteFor(round) {
  return reviewState.notes.get(noteKey(round)) ?? { round, tags: [], text: "" };
}

function createNoteEditor(round) {
  const note = { ...noteFor(round), tags: [...noteFor(round).tags] };
  const editor = element("div", "note");

  const tags = element("div", "tags");
  tags.setAttribute("role", "group");
  tags.setAttribute("aria-label", "課題のタグ");
  for (const tag of REVIEW_TAGS) {
    const button = element("button", "tag", tag);
    button.type = "button";
    button.setAttribute("aria-pressed", String(note.tags.includes(tag)));
    button.addEventListener("click", () => {
      note.tags = note.tags.includes(tag) ? note.tags.filter((entry) => entry !== tag) : [...note.tags, tag];
      button.setAttribute("aria-pressed", String(note.tags.includes(tag)));
    });
    tags.append(button);
  }

  const text = element("textarea");
  text.value = note.text;
  text.setAttribute("aria-label", round ? `第${round}ラウンドのメモ` : "試合全体のメモ");
  text.placeholder = round ? "何が起きて、次はどうするか" : "次の試合で一つだけ変えること";

  const status = element("span", "", "");
  status.setAttribute("role", "status");
  const save = element("button", "primary", "メモを保存");
  save.type = "button";
  save.addEventListener("click", async () => {
    note.text = text.value;
    const matchId = reviewState.review.match_id;
    try {
      await invoke("save_review_note", { matchId, note: { round, tags: note.tags, text: note.text } });
      reviewState.notes.set(noteKey(round), { round, tags: note.tags, text: note.text });
      reviewState.reviewed.add(matchId);
      status.textContent = "保存しました";
      riseIn(status);
      renderRounds();
      renderMatchList();
    } catch (error) {
      status.textContent = `保存できませんでした: ${error}`;
    }
  });

  const actions = element("div", "note-actions");
  actions.append(save, status);
  editor.append(tags, text, actions);
  return editor;
}

function renderRoundDetail() {
  const pane = document.getElementById("round-detail");
  const round = reviewState.review.rounds.find((entry) => entry.number === reviewState.round);
  if (!round) {
    pane.replaceChildren(element("p", "note-title", "ラウンドを選ぶと内訳とメモ欄を表示します"));
    return;
  }
  const facts = element("dl");
  fact(facts, "結果", round.won ? "勝ち" : "負け");
  fact(facts, "与ダメージ", round.damage);
  fact(facts, "装備の価値", round.loadout);
  const opening = round.first_kill ? "先手を取った" : round.first_death ? "最初に倒された" : "－";
  fact(facts, "開幕", opening);
  if (round.clutch_against) fact(facts, "クラッチ", `1v${round.clutch_against}`);
  if (round.plant_site) fact(facts, "設置", `${round.plant_site} サイト`);
  pane.replaceChildren(
    element("p", "note-title", `第${round.number}ラウンド`),
    (round.timeline ?? []).length > 0 ? roundTimeline(round) : eventList(round),
    facts,
    createNoteEditor(round.number),
  );
}

const EVENT_TEXT = {
  kill: (event) => `${event.target ?? "相手"} を倒した`,
  assist: (event) => `${event.target ?? "相手"} をアシスト`,
  death: (event) => `${event.target ?? "相手"} に倒された`,
  plant: () => "スパイクを設置",
  defuse: () => "スパイクを解除",
};

function formatRoundTime(ms) {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** ラウンドの中で自分がしたことを時間順に並べる。 */
function eventList(round) {
  const events = round.events ?? [];
  if (events.length === 0) return element("p", "fit-note", "記録された行動はありません。");
  const list = element("ol", "round-events");
  for (const event of events) {
    const item = element("li", `is-${event.kind}`);
    item.append(
      element("span", "event-time", formatRoundTime(event.at_ms)),
      event.target ? agentIcon(event.target, 26) : element("span", "event-mark"),
      element("span", "", EVENT_TEXT[event.kind](event)),
      element("span", "event-weapon", event.weapon ?? ""),
    );
    list.append(item);
  }
  return list;
}

const SIDE_TEXT = { own: "あなた", ally: "味方", enemy: "相手" };

function actorName(actor) {
  if (!actor) return "－";
  return actor.side === "own" ? "あなた" : `${actor.agent ?? "不明"}（${SIDE_TEXT[actor.side]}）`;
}

function timelineText(event) {
  if (event.kind === "plant") return `${actorName(event.actor)} がスパイクを設置`;
  if (event.kind === "defuse") return `${actorName(event.actor)} がスパイクを解除`;
  const assists = event.assists.length ? `（アシスト ${event.assists.map((a) => a.agent ?? "不明").join("、")}）` : "";
  return `${actorName(event.actor)} → ${actorName(event.target)}${assists}`;
}

/** ゲーム内の座標を、ミニマップの 0〜1000 の座標へ変える。 */
function toMinimap(point, transform) {
  return {
    x: (point.y * transform.x_multiplier + transform.x_scalar) * 1000,
    y: (point.x * transform.y_multiplier + transform.y_scalar) * 1000,
  };
}

/** 出来事の瞬間の全員の位置を、ミニマップに描く。 */
function minimapView(event) {
  const map = images.maps[reviewState.review.map];
  if (!map?.minimap || !map.transform || !event) return null;
  const board = svg("svg", { viewBox: "0 0 1000 1000", class: "minimap", role: "img",
    "aria-label": `${formatRoundTime(event.at_ms)} の位置。${timelineText(event)}` });
  board.append(svg("image", { href: map.minimap, x: 0, y: 0, width: 1000, height: 1000 }));
  for (const placed of event.positions) {
    const at = toMinimap(placed.at, map.transform);
    const group = svg("g", { class: `is-${placed.actor.side}` });
    if (placed.view !== null) {
      // 向きはゲーム内の +x 軸からの角度とみなす。ミニマップは x と y を入れ替えて描くので、成分も入れ替える。
      // 角度の基準は実データで未確認。
      const angle = placed.view / 1000;
      const dx = Math.sin(angle) * Math.sign(map.transform.x_multiplier) * 34;
      const dy = Math.cos(angle) * Math.sign(map.transform.y_multiplier) * 34;
      group.append(svg("line", { x1: at.x, y1: at.y, x2: at.x + dx, y2: at.y + dy, class: "view" }));
    }
    group.append(svg("circle", { cx: at.x, cy: at.y, r: 13 }));
    const title = svg("title", {});
    title.textContent = actorName(placed.actor);
    group.append(title);
    board.append(group);
  }
  if (event.location) {
    const at = toMinimap(event.location, map.transform);
    const mark = event.kind === "kill" ? "is-death-mark" : "is-spike-mark";
    board.append(svg("path", { d: `M${at.x - 14},${at.y - 14}L${at.x + 14},${at.y + 14}M${at.x + 14},${at.y - 14}L${at.x - 14},${at.y + 14}`, class: mark }));
  }
  return board;
}

/** ラウンドの全員の出来事。時間軸、一覧、選んだ瞬間のミニマップを並べる。 */
function roundTimeline(round) {
  const events = round.timeline;
  const selected = Math.min(reviewState.event ?? 0, events.length - 1);
  const wrap = element("div", "round-timeline");
  const end = Math.max(...events.map((event) => event.at_ms), 1);

  const axis = element("div", "timeline-axis");
  axis.setAttribute("aria-hidden", "true");
  events.forEach((event, index) => {
    const tick = element("i", `is-${event.kind} is-${event.actor?.side ?? "none"}${index === selected ? " is-selected" : ""}`);
    tick.style.left = `${(event.at_ms / end) * 100}%`;
    axis.append(tick);
  });

  const list = element("ol", "round-events timeline-events");
  events.forEach((event, index) => {
    const item = element("li", `is-${event.kind} is-${event.actor?.side ?? "none"}`);
    item.tabIndex = 0;
    item.setAttribute("role", "button");
    item.setAttribute("aria-pressed", String(index === selected));
    const choose = () => { reviewState.event = index; renderRoundDetail(); };
    item.addEventListener("click", choose);
    item.addEventListener("keydown", (key) => { if (key.key === "Enter" || key.key === " ") { key.preventDefault(); choose(); } });
    item.append(
      element("span", "event-time", formatRoundTime(event.at_ms)),
      event.actor?.agent ? agentIcon(event.actor.agent, 26) : element("span", "event-mark"),
      element("span", "", timelineText(event)),
      element("span", "event-weapon", [event.weapon, `${event.alive[0]}v${event.alive[1]}`].filter(Boolean).join("  ")),
    );
    list.append(item);
  });

  const map = minimapView(events[selected]);
  wrap.append(axis);
  if (map) wrap.append(map);
  wrap.append(list);
  return wrap;
}

function focusRound(number) {
  reviewState.round = number;
  reviewState.event = 0;
  renderRounds();
  renderRoundDetail();
  document.getElementById("round-strip")?.scrollIntoView({ block: "nearest" });
}

/** 試合の見どころ。最良の場面を大きく、続きを小さく出す。 */
function highlightSection(review) {
  const scenes = review.highlights ?? [];
  const section = element("section", "highlights");
  const head = element("div", "highlight-head");
  head.append(element("h2", "", "BEST SCENE / ベストシーン"));
  const share = element("button", "primary", "共有画像を作る");
  share.type = "button";
  share.addEventListener("click", () => openShare(review));
  head.append(share);
  section.append(head);
  if (scenes.length === 0) {
    section.append(element("p", "fit-note", "この試合に目立つ場面はありませんでした。"));
    return section;
  }
  const list = element("div", "highlight-list");
  scenes.forEach((scene, index) => {
    const button = element("button", index === 0 ? "scene is-best" : "scene");
    button.type = "button";
    button.append(element("span", "scene-title", scene.title), element("small", "", scene.detail));
    button.addEventListener("click", () => focusRound(scene.round));
    list.append(button);
  });
  section.append(list);
  return section;
}

function selectRound(number) {
  reviewState.round = reviewState.round === number ? null : number;
  reviewState.event = 0;
  renderRounds();
  renderRoundDetail();
}

function renderRounds() {
  const strip = document.getElementById("round-strip");
  if (!strip) return;
  strip.replaceChildren(...reviewState.review.rounds.map((round, index) => {
    const button = element("button", round.won ? "round is-won" : "round", round.number);
    button.style.setProperty("--index", index);
    button.type = "button";
    const opening = round.first_kill ? "、先手" : round.first_death ? "、最初に倒された" : "";
    button.setAttribute("aria-label", `第${round.number}ラウンド、${round.won ? "勝ち" : "負け"}${opening}`);
    button.setAttribute("aria-pressed", String(reviewState.round === round.number));
    if (reviewState.pointed.includes(round.number)) button.classList.add("is-pointed");
    if (round.first_death) button.append(element("i", "is-first-death"));
    if (round.first_kill) button.append(element("i", "is-first-kill"));
    if (reviewState.notes.has(round.number)) button.append(element("em"));
    if (round.kills >= 2) button.append(element("b", "round-kills", `${round.kills}K`));
    button.addEventListener("click", () => selectRound(round.number));
    return button;
  }));
}

function findingItem(finding) {
  const item = element("li");
  const title = element("b");
  title.append(element("span", `tone is-${finding.tone}`, TONE_LABELS[finding.tone]), finding.title);
  item.append(title, element("small", "", finding.detail));
  if (finding.rounds.length) {
    const show = element("button", "tag", "該当ラウンドを示す");
    show.type = "button";
    show.addEventListener("click", () => {
      reviewState.pointed = finding.rounds;
      reviewState.round = finding.rounds[0];
      renderRounds();
      renderRoundDetail();
    });
    item.append(show);
  }
  return item;
}

function scoreRow(line, isTeamStart) {
  const row = element("tr", [line.is_own ? "is-own" : "", isTeamStart ? "team-start" : ""].join(" ").trim());
  const name = element("td");
  const who = element("div", "score-player");
  who.append(agentIcon(line.agent ?? line.label, 24), line.label);
  name.append(who);
  const cell = (value) => valueCell(value);
  row.append(
    name, rankCell({ tier: line.tier, name: line.rank }), cell(line.acs), cell(line.kills), cell(line.deaths),
    cell(line.assists), cell(line.adr), cell(line.headshot_percent), cell(line.first_kills), cell(line.first_deaths),
  );
  return row;
}

function scoreboard(review) {
  const table = element("table");
  table.setAttribute("aria-label", "試合のスコア");
  const head = element("tr");
  SCORE_COLUMNS.forEach(([label, className], index) => {
    head.append(element("th", className, index === 0 ? "プレイヤー" : label));
  });
  const thead = element("thead");
  thead.append(head);
  const body = element("tbody");
  let previousAlly = null;
  review.lines.forEach((line, index) => {
    body.append(scoreRow(line, index > 0 && line.is_ally !== previousAlly));
    previousAlly = line.is_ally;
  });
  table.append(thead, body);
  const section = element("section", "scoreboard");
  section.append(element("h2", "", "SCOREBOARD / ほかの人はエージェント名で表示"), table);
  return section;
}

/** 試合後の順序: 勝敗、指標、ラウンドの流れ、見直す点。 */
function playReviewReveal(result, strip, side) {
  const score = result.previousElementSibling;
  wipeIn(score);
  [...result.children].forEach((node, index) => riseIn(node, { delay: 80 + index * 50 }));

  const roundsStart = motionReduced() ? 0 : 240;
  strip.style.setProperty("--rounds-start", `${roundsStart}ms`);
  strip.classList.add("is-revealing");
  // 描き直しでボタンが替わると animationend は届かないので、時間で解除する。
  setTimeout(() => strip.classList.remove("is-revealing"), REVEAL_MS);
  riseIn(side, { delay: 460 });
}

function renderReview(pane, review) {
  const outcome = review.won === null ? "Draw" : review.won ? "Win" : "Loss";
  const scoreClass = `result-score ${review.won === false ? "loss" : ""}`.trim();
  const score = displayText("h2", `${outcome} ${review.own_score}-${review.enemy_score}`, scoreClass);

  const own = review.own;
  const result = element("div", "result");
  result.append(
    figure(own.acs, `ACS / チーム ${review.team_acs}`),
    figure(own.adr, `ADR / チーム ${review.team_adr}`),
    figure(`${own.kills}/${own.deaths}/${own.assists}`, "K / D / A"),
    figure(own.headshot_percent === null ? "－" : `${own.headshot_percent}%`, "HEADSHOT"),
    figure(`${own.first_kills}:${own.first_deaths}`, "先手 : 被先手"),
  );
  const hero = element("section", "review-hero halftone");
  setHalftone(hero, review.map);
  hero.append(labelLine([review.map ?? "MATCH", own.rank]), score, result);

  const timeline = element("section");
  const strip = element("div", "rounds");
  strip.id = "round-strip";
  const key = element("p", "round-key label", "白は勝ち、黒は負け。下の印は開幕の結果、右上の点はメモありです");
  const detail = element("div");
  detail.id = "round-detail";
  timeline.append(element("h2", "", "ROUNDS / ラウンドの流れ"), strip, key, detail);

  const side = element("section");
  const findings = element("ol", "findings");
  findings.append(...review.findings.map(findingItem));
  side.append(element("h2", "", "FINDINGS / データから見える点"));
  side.append(review.findings.length ? findings : element("p", "note-title", "目立つ傾向はありませんでした"));
  side.append(element("h2", "whole-note", "NOTE / 試合全体のメモ"), createNoteEditor(null));

  const inner = element("div", "review-inner");
  inner.append(highlightSection(review), timeline, side, scoreboard(review));
  const panel = element("div");
  panel.id = "share-panel";
  const summary = review.match_id === afterMatch.matchId ? afterMatchPanel(review) : null;
  pane.replaceChildren(...[hero, summary, panel, inner].filter(Boolean));
  // 最初に見直す場所が分かるよう、先頭の気づきのラウンドを示す。
  const first = review.findings.find((finding) => finding.tone === "focus" && finding.rounds.length);
  if (first) {
    reviewState.pointed = first.rounds;
    reviewState.round = first.rounds[0];
  }
  renderRounds();
  renderRoundDetail();
  if (reviewState.justFinished) {
    reviewState.justFinished = false;
    playReviewReveal(result, strip, side);
  }
}

function renderMatchList() {
  const list = document.getElementById("match-list");
  if (!list) return;
  const history = [...(app.profile?.history ?? [])].reverse().filter((point) => point.match_id);
  const format = new Intl.DateTimeFormat("ja-JP", { month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit" });
  list.replaceChildren(...history.map((point) => {
    const item = element("li");
    const button = element("button");
    button.type = "button";
    button.setAttribute("aria-current", String(reviewState.opening === point.match_id));
    const written = reviewState.reviewed.has(point.match_id) ? " / メモあり" : "";
    const thumbnail = picture(images.maps[point.map]?.thumbnail);
    if (thumbnail) button.append(thumbnail);
    button.append(
      element("span", "", point.map ?? "マップ不明"),
      element("span", signClass(point.earned), formatSigned(point.earned)),
      element("small", "", `${format.format(new Date(point.at_ms))}${written}`),
    );
    button.addEventListener("click", () => openReview(point.match_id));
    item.append(button);
    return item;
  }));
}

function ensureReviewLayout(view) {
  if (document.getElementById("match-list")) return;
  const list = element("ol", "match-list");
  list.id = "match-list";
  list.setAttribute("aria-label", "試合の一覧");
  const pane = element("div", "review");
  pane.id = "review-pane";
  view.replaceChildren(list, pane);
}

function retryBox(matchId, detail) {
  const box = noticeBox("試合の詳細を取得できませんでした", detail);
  const again = element("button", "primary", "読み込み直す");
  again.type = "button";
  again.addEventListener("click", () => openReview(matchId));
  box.append(element("br"), element("br"), again);
  return box;
}

/** 取得できなければ間隔を空けて取り直し、上限に達したら手動の再読み込みを出す。 */
function scheduleRetry(matchId, pane, detail) {
  if (reviewState.attempt + 1 >= RETRY_LIMIT) {
    pane.replaceChildren(retryBox(matchId, detail));
    return;
  }
  reviewState.timer = setTimeout(() => openReview(matchId, reviewState.attempt + 1), RETRY_DELAY_MS);
}

async function openReview(matchId, attempt = 0) {
  clearTimeout(reviewState.timer);
  reviewState.attempt = attempt;
  // 後から届いた古い試合の応答で、選び直した画面を上書きしない。
  reviewState.requestId += 1;
  const requestId = reviewState.requestId;
  reviewState.opening = matchId;
  // 別の試合を開いたら、試合直後のまとめは閉じる。
  if (matchId !== afterMatch.matchId) afterMatch.matchId = null;

  const view = document.getElementById("review-view");
  ensureReviewLayout(view);
  renderMatchList();
  const pane = document.getElementById("review-pane");
  pane.replaceChildren(noticeBox("試合を集計しています", "Riot の集計が終わると表示します。"));

  try {
    const [review, notes] = await Promise.all([
      invoke("load_review", { matchId }),
      invoke("review_notes", { matchId }),
    ]);
    if (requestId !== reviewState.requestId) return;
    if (!review) {
      scheduleRetry(matchId, pane, "時間をおいて読み込み直してください。");
      return;
    }
    reviewState.review = review;
    reviewState.round = null;
    reviewState.pointed = [];
    reviewState.notes = new Map(notes.map((note) => [noteKey(note.round), note]));
    renderReview(pane, review);
    renderMatchList();
  } catch (error) {
    if (requestId === reviewState.requestId) scheduleRetry(matchId, pane, String(error));
  }
}

async function showReviewList() {
  ensureReviewLayout(document.getElementById("review-view"));
  reviewState.reviewed = new Set(await invoke("reviewed_matches"));
  renderMatchList();
  if (!reviewState.review && reviewState.opening === null) {
    const latest = app.profile?.history.at(-1)?.match_id;
    if (latest) openReview(latest);
    else {
      document.getElementById("review-pane").replaceChildren(
        noticeBox("振り返る試合はまだありません", "コンペティティブを遊ぶと、ここに試合が並びます。"),
      );
    }
  }
}

function openReviewFromHome(matchId) {
  showView("review");
  openReview(matchId);
}
