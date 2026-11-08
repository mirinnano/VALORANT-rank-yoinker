const CHART = { width: 760, height: 260, left: 84, right: 16, top: 16, bottom: 24 };
const SVG_NS = "http://www.w3.org/2000/svg";
/** 新しい試合の結果が入ったときだけ、線を左から見せる。 */
const homeReveal = { pending: true };

function svg(tag, attributes, text) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
  if (text !== undefined) node.textContent = text;
  return node;
}

/** tier と RR を 1 本の軸へ載せる。1 tier は 100 RR に相当する。 */
function rating(point) {
  return point.tier * 100 + point.rr;
}

function createChart(history) {
  const { width, height, left, right, top, bottom } = CHART;
  const values = history.map(rating);
  const low = Math.floor(Math.min(...values) / 100) * 100;
  const high = Math.ceil((Math.max(...values) + 1) / 100) * 100;
  const scaleY = (value) => top + ((high - value) / (high - low)) * (height - top - bottom);
  const step = history.length > 1 ? (width - left - right) / (history.length - 1) : 0;
  const scaleX = (index) => left + index * step;

  const first = history[0];
  const last = history[history.length - 1];
  const chart = svg("svg", {
    class: "chart",
    viewBox: `0 0 ${width} ${height}`,
    role: "img",
    "aria-label": `直近 ${history.length} 試合の推移。${first.rank_name} から ${last.rank_name} へ。`,
  });

  for (let value = low; value <= high; value += 100) {
    const y = scaleY(value);
    chart.append(svg("line", { class: "grid", x1: left, x2: width - right, y1: y, y2: y }));
    const point = history.find((entry) => entry.tier === value / 100);
    chart.append(svg("text", { class: "label", x: 0, y: y + 4 }, point ? point.rank_name : ""));
  }

  const points = history.map((point, index) => `${scaleX(index)},${scaleY(rating(point))}`);
  chart.append(svg("polyline", { class: "line", points: points.join(" ") }));
  chart.append(svg("circle", {
    class: "dot", cx: scaleX(history.length - 1), cy: scaleY(rating(last)), r: 4,
  }));

  const frame = element("div", "chart-frame");
  const tip = element("div", "chart-tip");
  tip.hidden = true;
  const guide = svg("line", { class: "guide", y1: top, y2: height - bottom });
  const focus = svg("circle", { class: "focus", r: 4 });

  const showPoint = (point, index) => {
    const x = scaleX(index);
    const y = scaleY(rating(point));
    guide.setAttribute("x1", x);
    guide.setAttribute("x2", x);
    focus.setAttribute("cx", x);
    focus.setAttribute("cy", y);
    guide.classList.add("is-active");
    focus.classList.add("is-active");
    tip.replaceChildren(`${formatSigned(point.earned)} RR`, element("small", "", `${point.map ?? "マップ不明"}、${point.rank_name}`));
    tip.hidden = false;
    // 右端では左側へ出し、グラフの外へはみ出さないようにする。
    const flip = x / width > 0.75 ? " translateX(-100%)" : "";
    tip.style.transform = `translate(${(x / width) * frame.clientWidth + (flip ? -12 : 12)}px, 8px)${flip}`;
  };

  history.forEach((point, index) => {
    const hit = svg("rect", {
      class: "hit", x: scaleX(index) - step / 2, y: top, width: step || 20, height: height - top - bottom,
    });
    hit.addEventListener("mouseenter", () => showPoint(point, index));
    chart.append(hit);
  });
  chart.append(guide, focus);
  chart.addEventListener("mouseleave", () => {
    tip.hidden = true;
    guide.classList.remove("is-active");
    focus.classList.remove("is-active");
  });
  frame.append(chart, tip);
  return frame;
}

function signClass(value) {
  if (value > 0) return "gain";
  return value < 0 ? "loss" : "";
}

function figure(value, label, className = "") {
  const box = element("div", "figure");
  box.append(element("b", className, value), label);
  return box;
}

function matchOutcome(entry) {
  if (entry.rr_earned !== null) return [`${formatSigned(entry.rr_earned)} RR`, signClass(entry.rr_earned)];
  if (entry.won === true) return ["WIN", "gain"];
  if (entry.won === false) return ["LOSS", "loss"];
  return [entry.kills === null ? "－" : `${entry.kills}/${entry.deaths}/${entry.assists}`, ""];
}

// 全モードの最近の試合。ランク戦は RR、それ以外は勝敗を大きく出す。
function recentList(matches) {
  const list = element("ol", "recent");
  const format = new Intl.DateTimeFormat("ja-JP", { month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit" });
  for (const entry of matches.slice(0, 8)) {
    const item = element("li");
    item.title = "振り返りを開く";
    item.classList.add("is-link");
    item.tabIndex = 0;
    item.setAttribute("role", "button");
    const open = () => openReviewFromHome(entry.match_id);
    item.addEventListener("click", open);
    item.addEventListener("keydown", (event) => { if (event.key === "Enter") open(); });
    const thumbnail = picture(images.maps[entry.map]?.thumbnail);
    if (thumbnail) item.append(thumbnail);
    const [outcome, outcomeClass] = matchOutcome(entry);
    const detail = [entry.mode, entry.agent, entry.acs === null ? null : `ACS ${entry.acs}`].filter(Boolean).join(" / ");
    item.append(
      element("span", "", entry.map ?? entry.mode ?? "試合"),
      element("span", outcomeClass, outcome),
      element("small", "", `${format.format(new Date(entry.at_ms))}${detail ? `  ${detail}` : ""}`),
    );
    list.append(item);
  }
  return list;
}

async function loadTagTrend(container) {
  const tags = await window.__TAURI__.core.invoke("tag_trend");
  if (tags.length === 0) return;
  const list = element("ol", "trend");
  for (const { tag, count } of tags.slice(0, 5)) {
    const item = element("li");
    item.append(element("span", "", tag), element("span", "", `${count}回`));
    list.append(item);
  }
  container.replaceChildren(element("h2", "", "REPEATED / 振り返りで多い課題"), list);
}

function renderHome(view, profile) {
  if (!profile) {
    view.replaceChildren(noticeBox("プロフィールを読み込んでいます", "取得できない場合は、VALORANT を開いた状態で少し待ってください。"));
    return;
  }

  const reveal = homeReveal.pending;
  const hero = element("section", "home-hero halftone");
  setHalftone(hero, profile.history.at(-1)?.map);
  const disc = element("div", "hero-disc");
  disc.append(rankIcon(profile.rank.tier, 148, "large"));
  hero.append(disc);

  const peak = profile.peak_act ? `${profile.peak.name} (${profile.peak_act})` : profile.peak.name;
  const figures = element("div", "hero-figures");
  figures.append(
    figure(formatSigned(profile.last_day_rr), `24H RR / ${profile.last_day_games}試合`, signClass(profile.last_day_rr)),
    figure(profile.win_rate === null ? "－" : `${profile.win_rate}%`, `今シーズン勝率 / ${profile.games}試合`),
    figure(peak, "PEAK"),
  );
  const rankTitle = displayText("h2", profile.rank.name, "hero-rank");
  hero.append(
    labelLine([profile.name ?? "PLAYER", "COMPETITIVE", profile.rank.tier > 0 ? `${profile.rank.rr} RR` : null]),
    rankTitle,
    figures,
  );

  const home = element("div", "home");
  home.append(sessionBlock(profile, "home-session"));
  if (profile.history.length < 2) {
    home.append(noticeBox("RR の推移はまだ表示できません", "コンペティティブを2試合遊ぶと、ここに RR の推移が表示されます。"));
  } else {
    const trend = element("section");
    const chart = createChart(profile.history);
    if (homeReveal.pending) {
      homeReveal.pending = false;
      chart.querySelector(".chart").classList.add("is-revealing");
    }
    trend.append(element("h2", "", `RR TREND / 直近${profile.history.length}試合`), chart);
    home.append(trend);
  }
  const recent = element("section");
  if (profile.matches.length > 0) {
    recent.append(element("h2", "", `RECENT / 最近の${profile.matches.length}試合`), recentList(profile.matches));
  } else if (profile.partial) {
    recent.append(noticeBox("試合の記録を読み込めませんでした", "少し待ってから自動で取り直します。続く場合は設定の診断を実行してください。"));
  }
  const repeated = element("div");
  repeated.id = "tag-trend";
  recent.append(repeated);
  home.append(recent);
  loadTagTrend(repeated);
  view.replaceChildren(hero, home);
  if (reveal) wipeIn(rankTitle);
}
