// 試合が終わったときに、振り返りの先頭へ出すまとめ。結果、RR、今日の成績、見せ場のクリップを1か所に集める。
const AFTER_MATCH_CLIPS = 3;
const afterMatch = { matchId: null, review: null };

function afterResult(review) {
  if (review.won === null) return ["DRAW", ""];
  return review.won ? ["WIN", "gain"] : ["LOSS", "loss"];
}

/** RR は Riot の集計が終わってから届くので、届くまでは集計中と出す。 */
function afterRrFigure(matchId) {
  const entry = app.profile?.matches?.find((item) => item.match_id === matchId);
  if (!entry) return figure("集計中", "RR");
  if (entry.rr_earned === null) return figure("－", entry.mode ?? "RR");
  return figure(formatSigned(entry.rr_earned), "RR", signClass(entry.rr_earned));
}

function afterRankFigure() {
  const rank = app.profile?.rank;
  if (!rank || rank.tier === 0) return null;
  return figure(`${rank.name} ${rank.rr}`, "現在のランク / RR");
}

function afterSessionLine() {
  if (!app.profile) return null;
  const day = todaySession(app.profile.matches);
  if (day.games === 0) return null;
  const rr = day.ranked ? `、${formatSigned(day.rr)} RR` : "";
  return element("p", "after-session", `今日 ${day.games}試合 ${day.wins}勝${day.losses}敗${rr}`);
}

function afterFigures(review) {
  const own = review.own;
  const [outcome, outcomeClass] = afterResult(review);
  const figures = element("div", "hero-figures after-figures");
  figures.id = "after-figures";
  figures.append(
    figure(`${outcome} ${review.own_score}-${review.enemy_score}`, review.map ?? "試合", outcomeClass),
    afterRrFigure(review.match_id),
    ...[afterRankFigure()].filter(Boolean),
    figure(`${own.kills}/${own.deaths}/${own.assists}`, `K/D/A / ACS ${own.acs}`),
  );
  return figures;
}

function afterClipRow(recording, clip) {
  const item = element("li", `clip is-${clip.kind}`);
  const where = clip.round ? `第${clip.round}ラウンド` : "保存した場面";
  const meta = element("span", "clip-meta", where);
  const share = element("button", "primary", "共有用に書き出す");
  share.type = "button";
  share.disabled = !recording.exists;
  share.addEventListener("click", () => exportShareClip(recording, clip, share, meta));
  item.append(element("span", "clip-title", clip.title), meta, share);
  return item;
}

/** 録画がなければ何も出さない。切り出しは試合の詳細が届いてから進むので、届くたびに描き直す。 */
async function fillAfterClips(container, matchId) {
  let recordings;
  try {
    recordings = await invoke("list_recordings");
  } catch {
    return;
  }
  if (afterMatch.matchId !== matchId) return;
  const recording = recordings.find((item) => item.match_id === matchId);
  if (!recording) {
    container.replaceChildren();
    return;
  }
  const clips = [...recording.clips]
    .filter((clip) => clip.kind !== "manual")
    .sort((a, b) => (CLIP_ORDER[a.kind] - CLIP_ORDER[b.kind]) || (b.kills - a.kills) || (a.start_ms - b.start_ms))
    .slice(0, AFTER_MATCH_CLIPS);
  const head = element("div", "after-clips-head");
  head.append(element("h3", "", "HIGHLIGHTS / 見せ場"));
  const open = element("button", "tag", "クリップ画面を開く");
  open.type = "button";
  open.addEventListener("click", () => showView("clips"));
  head.append(open);
  if (clips.length === 0) {
    container.replaceChildren(head, element("p", "lobby-note", "録画から見せ場を切り出しています。終わるとここに並びます。"));
    return;
  }
  const list = element("ol", "clip-list after-clips");
  list.append(...clips.map((clip) => afterClipRow(recording, clip)));
  container.replaceChildren(head, list);
}

function afterMatchPanel(review) {
  afterMatch.review = review;
  const panel = element("section", "after-match");
  panel.setAttribute("aria-label", "試合のまとめ");
  const head = element("div", "after-head");
  head.append(element("h2", "", "AFTER MATCH / 試合のまとめ"));
  const share = element("button", "primary", "共有画像を作る");
  share.type = "button";
  share.addEventListener("click", () => openShare(review));
  head.append(share);
  const clips = element("div", "after-clips-box");
  panel.append(...[head, afterFigures(review), afterSessionLine(), clips].filter(Boolean));
  fillAfterClips(clips, review.match_id);
  return panel;
}

/** プロフィールが更新されたら、RR と今日の成績だけを差し替える。 */
function refreshAfterMatch() {
  const panel = document.querySelector(".after-match");
  if (!panel || !afterMatch.review) return;
  panel.querySelector("#after-figures")?.replaceWith(afterFigures(afterMatch.review));
  const line = afterSessionLine();
  const old = panel.querySelector(".after-session");
  if (old && line) old.replaceWith(line);
  else if (line) panel.querySelector(".after-clips-box")?.before(line);
}

window.__TAURI__.event.listen("clips-updated", () => {
  const box = document.querySelector(".after-match .after-clips-box");
  if (box && afterMatch.matchId) fillAfterClips(box, afterMatch.matchId);
});
