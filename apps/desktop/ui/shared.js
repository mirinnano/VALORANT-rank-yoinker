// tier は 3 から 27 まで。8 ランクが 3 段ずつあり、最後が Radiant。
const RANK_BANDS = [
  ["Iron", "#8d8678"], ["Bronze", "#bb8f5a"], ["Silver", "#aeb2b2"], ["Gold", "#e8c84a"],
  ["Platinum", "#3fb8c9"], ["Diamond", "#d864c7"], ["Ascendant", "#27b56c"],
  ["Immortal", "#ff4a5a"], ["Radiant", "#fffdcd"],
];
const FIRST_TIER = 3;
const TIERS_PER_BAND = 3;
const UNRANKED_COLOR = "#59636f";

const images = { agents: {}, maps: {}, tiers: {} };
const motionReduced = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";

function bandOf(tier) {
  if (tier < FIRST_TIER) return -1;
  return Math.min(Math.floor((tier - FIRST_TIER) / TIERS_PER_BAND), RANK_BANDS.length - 1);
}

function tierColor(tier) {
  const band = bandOf(tier);
  return band < 0 ? UNRANKED_COLOR : RANK_BANDS[band][1];
}

/** `Gold 3` のような名前から tier を求める。履歴の行で使う。 */
function tierOfName(name) {
  const [band, step] = String(name).split(" ");
  const index = RANK_BANDS.findIndex(([label]) => label === band);
  if (index < 0) return 0;
  return FIRST_TIER + index * TIERS_PER_BAND + Math.max(Number(step || 1) - 1, 0);
}

/** 試合の節目で画面に入る動き。手動の切り替えでは使わない。 */
function enterView(node) {
  const from = motionReduced() ? { opacity: 0 } : { opacity: 0, transform: "translateY(8px)" };
  const to = motionReduced() ? { opacity: 1 } : { opacity: 1, transform: "none" };
  node.animate([from, to], { duration: 240, easing: EASE_OUT });
}

const TRAVEL = { none: "translateY(0)", rise: "translateY(4px)" };

/** 小さく上昇して現れる。動きを減らす設定では不透明度だけにする。 */
function riseIn(node, { delay = 0, duration = 180 } = {}) {
  const from = motionReduced() ? { opacity: 0 } : { opacity: 0, transform: TRAVEL.rise };
  const to = motionReduced() ? { opacity: 1 } : { opacity: 1, transform: TRAVEL.none };
  node.animate([from, to], { duration, delay: motionReduced() ? 0 : delay, easing: EASE_OUT, fill: "backwards" });
}

/** 大きな見出しを左から現し、最後に赤い四角を置く。試合の節目だけで使う。 */
function wipeIn(node, { delay = 0 } = {}) {
  if (motionReduced()) {
    node.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 180, easing: "ease" });
    return;
  }
  node.animate(
    [{ clipPath: "inset(-10% 100% -10% -5%)" }, { clipPath: "inset(-10% -5% -10% -5%)" }],
    { duration: 360, delay, easing: EASE_OUT, fill: "backwards" },
  );
  node.querySelector(".period")?.animate(
    [{ opacity: 0, transform: "scale(0.6)" }, { opacity: 1, transform: "scale(1)" }],
    { duration: 120, delay: delay + 280, easing: EASE_OUT, fill: "backwards" },
  );
}

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = text;
  return node;
}

/** 画像は装飾か、隣の文字と同じ内容なので alt は空にする。読み込めなければ消す。 */
function picture(url, className, size) {
  if (!url) return null;
  const image = element("img", className);
  image.alt = "";
  image.decoding = "async";
  image.src = url;
  if (size) {
    image.width = size;
    image.height = size;
  }
  image.addEventListener("error", () => image.remove());
  return image;
}

function agentIcon(name, size) {
  const image = picture(images.agents[name]?.icon, "agent-icon", size);
  if (image) return image;
  const blank = element("span", "icon-blank");
  blank.style.width = `${size}px`;
  blank.style.height = `${size}px`;
  return blank;
}

function rankIcon(tier, size, variant = "small") {
  const image = picture(images.tiers[tier]?.[variant], "rank-icon", size);
  if (image) return image;
  const blank = element("span", "icon-blank");
  blank.style.width = `${size}px`;
  blank.style.height = `${size}px`;
  blank.style.background = tierColor(tier);
  return blank;
}

/** 網点の背景を差し替える。画像がなければ墨色のまま。 */
function setHalftone(node, mapName) {
  node.querySelector(":scope > .halftone-image")?.remove();
  const image = picture(images.maps[mapName]?.splash, "halftone-image");
  if (image) node.prepend(image);
}

/** 見出しの末尾に赤い四角を置く。 */
function displayText(tag, text, className = "") {
  const node = element(tag, `display ${className}`.trim(), text);
  node.append(element("span", "period"));
  return node;
}

function labelLine(parts) {
  const line = element("p", "label");
  parts.filter(Boolean).forEach((part, index) => {
    if (index > 0) line.append(" ", element("b", "", "/"), " ");
    line.append(part);
  });
  return line;
}

/** 直近の試合を古い順に左から並べる。勝ちは白、負けは赤、増減なしは灰色。 */
function formPips(recent) {
  const pips = element("span", "form");
  pips.setAttribute("aria-label", `直近${recent.length}試合、${recent.filter((rr) => rr > 0).length}勝`);
  for (const earned of [...recent].reverse()) {
    pips.append(element("i", earned > 0 ? "is-win" : earned < 0 ? "is-loss" : ""));
  }
  return pips;
}

/** 自分の直近の履歴から、このマップの成績を求める。 */
function mapRecord(profile, map) {
  const games = (profile?.history ?? []).filter((point) => point.map === map);
  if (!map || games.length === 0) return null;
  const wins = games.filter((point) => point.earned > 0).length;
  return `直近の自分 ${wins}勝${games.length - wins}敗`;
}

function formatSigned(value) {
  return value > 0 ? `+${value}` : String(value).replace("-", "−");
}

function partyLetter(number) {
  return number ? String.fromCharCode(64 + number) : "";
}
