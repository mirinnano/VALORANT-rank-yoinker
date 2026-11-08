// キルの演出「ミスプリント」。数字を映像の上へ刷る。黄、赤、紙の 3 版が離れた位置から
// 叩きつけられ、少しずれたまま止まる。倒した相手のエージェントを順に消し込み、
// ラウンド、武器、何秒で何キルかを 1 行で添える。クリップの再生と共有用の書き出しの両方に使う。
const REACTION_IMAGE_KEY = "kill-reaction-image";
const REACTION_SIZE = 384;
/** 演出を出しておく時間。数が大きいほど長く見せる。 */
const REACTION_HOLD_MS = { 1: 1100, 2: 1300, 3: 1600, 4: 1800, 5: 2400 };
const REACTION_LABEL = { 1: "KILL", 2: "2K", 3: "3K", 4: "4K", 5: "ACE" };
/** 刷り上がりで残す版ずれ。紙の版に対する、黄と赤の位置。 */
const PLATE_REST = { yellow: [9, 7], red: [-5, -4] };
/** 版が飛んでくる前の位置。ここから定位置へ叩きつける。 */
const PLATE_FROM = { yellow: [70, 40], red: [-60, -30], paper: [0, -24] };

// 曲線はアプリの --ease-out と、着地用の Bounce settle の 2 本に絞る。
const RX_EASE = "cubic-bezier(0.23, 1, 0.32, 1)";
const RX_SLAM = "cubic-bezier(0.175, 0.885, 0.32, 1.275)";

const reaction = { image: null, halftone: null, timer: null, frame: null, shown: new Set() };

function readReactionImage() {
  try {
    return localStorage.getItem(REACTION_IMAGE_KEY);
  } catch {
    return null;
  }
}

function reducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** 大きな画像は保存できる大きさに縮める。 */
function shrinkImage(file) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const image = new Image();
    image.onload = () => {
      const scale = Math.min(1, REACTION_SIZE / Math.max(image.width, image.height));
      const canvas = document.createElement("canvas");
      canvas.width = Math.round(image.width * scale);
      canvas.height = Math.round(image.height * scale);
      canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height);
      URL.revokeObjectURL(url);
      resolve(canvas.toDataURL("image/png"));
    };
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("画像を読み込めません"));
    };
    image.src = url;
  });
}

/** data URL の画像を読む。loadImage は URL に問い合わせ文字列を足すので、data URL には使えない。 */
function loadDataImage(source) {
  return new Promise((resolve) => {
    if (!source) {
      resolve(null);
      return;
    }
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => resolve(null);
    image.src = source;
  });
}

/**
 * 選んだ画像を、ブランドの紙色の網点に刷り直す。明るいところほど大きな点になる。
 * どんな画像でも誌面の一部に見えるようにするため。
 */
async function halftoneImage(source, size = 240) {
  const image = await loadDataImage(source);
  if (!image) return null;
  const sample = document.createElement("canvas");
  sample.width = size;
  sample.height = size;
  const sampler = sample.getContext("2d", { willReadFrequently: true });
  const fit = Math.max(size / image.width, size / image.height);
  sampler.drawImage(image, (size - image.width * fit) / 2, (size - image.height * fit) / 2, image.width * fit, image.height * fit);
  const pixels = sampler.getImageData(0, 0, size, size).data;
  const out = document.createElement("canvas");
  out.width = size;
  out.height = size;
  const context = out.getContext("2d");
  const cell = 6;
  context.fillStyle = "#f2f1ec";
  for (let y = cell / 2; y < size; y += cell) {
    for (let x = cell / 2; x < size; x += cell) {
      const index = (Math.floor(y) * size + Math.floor(x)) * 4;
      const alpha = pixels[index + 3] / 255;
      const light = ((pixels[index] * 0.299 + pixels[index + 1] * 0.587 + pixels[index + 2] * 0.114) / 255) * alpha;
      const radius = (cell / 2) * Math.sqrt(light) * 1.05;
      if (radius < 0.3) continue;
      context.beginPath();
      context.arc(x, y, radius, 0, Math.PI * 2);
      context.fill();
    }
  }
  return out.toDataURL("image/png");
}

async function reactionHalftone() {
  const source = reaction.image ?? readReactionImage();
  if (!source) return null;
  if (reaction.halftone?.source !== source) reaction.halftone = { source, data: await halftoneImage(source) };
  return reaction.halftone.data;
}

function reactionLevel(count) {
  return Math.min(Math.max(count, 1), 5);
}

/** `kills` の中で `index` 番目のキルまでの、同じラウンドの連続キル。 */
function streakAt(kills, index) {
  const current = kills[index];
  const streak = kills.slice(0, index + 1).filter((kill) => kill.round === current.round);
  return {
    count: streak.length,
    victims: streak.map((kill) => ({ agent: kill.victim, headshot: Boolean(kill.headshot) })),
    first: streak[0].at_ms,
    current,
  };
}

const SKULL_PATH =
  "M12 2C6.9 2 3 5.7 3 10.4c0 2.7 1.2 5 3.2 6.4V19c0 .6.4 1 1 1h1.4v-2.3h1.7V20h1.8v-2.3h1.8V20h1.8v-2.3h1.7V20H17c.6 0 1-.4 1-1v-2.2c2-1.4 3-3.7 3-6.4C21 5.7 17.1 2 12 2z";

/** ヘッドショットの印。絵文字ではなく、描いた頭蓋骨を使う。 */
function skullIcon() {
  const ns = "http://www.w3.org/2000/svg";
  const icon = document.createElementNS(ns, "svg");
  icon.setAttribute("viewBox", "0 0 24 24");
  icon.setAttribute("class", "rx-skull");
  icon.setAttribute("aria-hidden", "true");
  const shape = document.createElementNS(ns, "path");
  shape.setAttribute("d", SKULL_PATH);
  icon.append(shape);
  for (const [cx, cy, r] of [[8.6, 10.6, 2.4], [15.4, 10.6, 2.4]]) {
    const eye = document.createElementNS(ns, "circle");
    eye.setAttribute("cx", cx);
    eye.setAttribute("cy", cy);
    eye.setAttribute("r", r);
    eye.setAttribute("class", "rx-skull-eye");
    icon.append(eye);
  }
  const nose = document.createElementNS(ns, "path");
  nose.setAttribute("d", "M12 13.4l-1.4 2.4h2.8z");
  nose.setAttribute("class", "rx-skull-eye");
  icon.append(nose);
  return icon;
}

// キルログの文言。ゲームのキルログ風に英語で書く。同じキルには毎回同じ文を出すよう、
// 時刻から選ぶ。{v} は相手、{w} は武器。
const KILL_LINES = {
  headshot: [
    "{v} took a {w} to the dome",
    "{v} got one-tapped by {w}",
    "{v}'s skull met a {w} round",
    "{v} caught a {w} between the eyes",
  ],
  sniper: [
    "{v} was picked off by {w}",
    "{v} peeked straight into a {w}",
    "{v} was deleted from across the map by {w}",
  ],
  shotgun: [
    "{v} was blown away by {w}",
    "{v} ate a point-blank {w}",
  ],
  melee: [
    "{v} was sliced and diced",
    "{v} got knifed from behind",
  ],
  ability: [
    "{v} was erased by an ability",
    "{v} was vaporized",
  ],
  default: [
    "{v} was turned to scrap by {w}",
    "{v} was sent to the shadow realm by {w}",
    "{v} got rekt by {w}",
    "{v} was absolutely clapped by {w}",
    "{v} was forcibly logged out by {w}",
    "{v} became {w} target practice",
  ],
};
const SNIPERS = ["operator", "marshal", "outlaw"];
const SHOTGUNS = ["judge", "bucky", "shorty"];

function killGroup(kill) {
  const name = (kill.weapon ?? "").toLowerCase();
  if (kill.headshot) return "headshot";
  if (!name) return "ability";
  if (name.includes("melee") || name.includes("knife") || name.includes("近接")) return "melee";
  if (SNIPERS.some((key) => name.includes(key))) return "sniper";
  if (SHOTGUNS.some((key) => name.includes(key))) return "shotgun";
  return "default";
}

function killLine(kill) {
  const lines = KILL_LINES[killGroup(kill)];
  const pick = lines[Math.abs(Math.floor(kill.at_ms / 7)) % lines.length];
  return pick.replace("{v}", kill.victim ?? "Enemy").replace("{w}", kill.weapon || "an ability");
}

/** 添える 1 行。何ラウンド目に、何で、何秒で何キルか。 */
function streakLine(streak) {
  const seconds = (streak.current.at_ms - streak.first) / 1000;
  const pace = streak.count >= 2 ? `${streak.count} KILLS IN ${seconds.toFixed(1)}S` : null;
  return [streak.current.round ? `R${streak.current.round}` : null, streak.current.weapon?.toUpperCase(), pace].filter(Boolean).join(" / ");
}

/** プレイヤーの上に重ねる層。なければ作る。 */
function reactionLayer() {
  const stage = document.querySelector(".clip-stage") ?? document.body;
  let layer = document.getElementById("reaction-layer");
  if (layer && layer.parentElement === stage) return layer;
  layer?.remove();
  layer = element("div", stage === document.body ? "reaction-layer is-floating" : "reaction-layer");
  layer.id = "reaction-layer";
  layer.setAttribute("aria-live", "polite");
  stage.append(layer);
  return layer;
}

/** 倒した相手の並び。最後の 1 体だけ、今まさに撃ち抜いて消し込む。弾の跡は頭か胴に残る。 */
function victimTally(victims) {
  const tally = element("div", "rx-tally");
  victims.forEach((victim, index) => {
    const fresh = index === victims.length - 1;
    const slot = element("span", ["rx-victim", fresh ? "is-new" : "", victim.headshot ? "is-headshot" : ""].join(" ").trim());
    const portrait = element("span", "rx-face-icon");
    portrait.append(agentIcon(victim.agent, 40), element("i", "rx-hole"));
    slot.append(portrait, element("i", "rx-strike"));
    if (victim.headshot) slot.append(skullIcon());
    if (fresh) slot.append(element("i", "rx-tracer"), element("i", "rx-impact"));
    slot.title = [victim.agent, victim.headshot ? "ヘッドショット" : null].filter(Boolean).join("、");
    tally.append(slot);
  });
  return tally;
}

/** 刷る文字。3 版を重ね、後で別々に動かす。 */
function printedLabel(text) {
  const print = element("div", "rx-print");
  print.setAttribute("aria-label", text);
  for (const plate of ["yellow", "red", "paper"]) {
    const layer = element("b", `rx-plate is-${plate}`, text);
    layer.setAttribute("aria-hidden", "true");
    print.append(layer);
  }
  return print;
}

function reactionNode(streak, halftone) {
  const level = reactionLevel(streak.count);
  const root = element("div", `rx rx-l${level}`);
  root.append(element("i", "rx-flash"));
  if (level === 1) {
    // 1 キルは左下のキルログ。数字は小さく刷り、文言を主役にする。
    const log = element("div", "rx-log");
    const words = element("p", "rx-message");
    for (const word of killLine(streak.current).split(" ")) words.append(element("span", "", word), " ");
    const text = element("div", "rx-log-text");
    text.append(printedLabel("KILL"), words, element("p", "rx-line", streakLine(streak)));
    log.append(victimTally(streak.victims), text);
    root.append(log);
    return root;
  }
  const band = element("div", "rx-band");
  if (halftone) {
    const portrait = element("img", "rx-portrait");
    portrait.alt = "";
    portrait.src = halftone;
    band.append(portrait);
  }
  const copy = element("div", "rx-copy");
  copy.append(
    printedLabel(REACTION_LABEL[level]),
    element("p", "rx-message", killLine(streak.current)),
    element("p", "rx-line", streakLine(streak)),
    victimTally(streak.victims),
  );
  band.append(copy);
  root.append(band);
  return root;
}

/** 入りの振り付け。帯が斜めに切り込み、版が順に叩きつけられ、最後の相手を消し込む。 */
function playReactionIn(root, level) {
  const flash = root.querySelector(".rx-flash");
  if (reducedMotion()) {
    root.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 160, easing: "ease-out", fill: "both" });
    return;
  }
  flash.animate([{ opacity: level >= 3 ? 0.5 : 0.22 }, { opacity: 0 }], { duration: level >= 3 ? 320 : 180, easing: RX_EASE, fill: "both" });
  const band = root.querySelector(".rx-band, .rx-log");
  band?.animate([{ clipPath: "inset(0 100% 0 0)" }, { clipPath: "inset(0 0 0 0)" }], { duration: 220, easing: RX_EASE, fill: "both" });
  // 1 キルの文言は一語ずつ、左から流れ込む。2K 以上は 1 行まとめて出す。
  const words = [...root.querySelectorAll(".rx-message span")];
  words.forEach((word, index) => {
    word.animate([{ opacity: 0, transform: "translateX(-12px)" }, { opacity: 1, transform: "translateX(0)" }], {
      duration: 180, delay: 260 + index * 60, easing: RX_EASE, fill: "both",
    });
  });
  if (words.length === 0) {
    root.querySelector(".rx-message")?.animate([{ opacity: 0, transform: "translateY(8px)" }, { opacity: 1, transform: "translateY(0)" }], {
      duration: 200, delay: 260, easing: RX_EASE, fill: "both",
    });
  }
  band?.querySelector(".rx-portrait")?.animate(
    [{ transform: "translateX(-12%) scale(1.15)", opacity: 0 }, { transform: "translateX(0) scale(1)", opacity: 1 }],
    { duration: 520, delay: 60, easing: RX_EASE, fill: "both" },
  );
  // 版は黄、赤、紙の順に飛んでくる。紙の版が着いた瞬間が一番強い。
  ["yellow", "red", "paper"].forEach((plate, index) => {
    const node = root.querySelector(`.rx-plate.is-${plate}`);
    if (!node) return;
    const [fromX, fromY] = PLATE_FROM[plate];
    const [restX, restY] = PLATE_REST[plate] ?? [0, 0];
    node.animate(
      [
        { opacity: 0, transform: `translate(${fromX}px, ${fromY}px) scale(1.35)` },
        { opacity: 1, transform: `translate(${restX}px, ${restY}px) scale(1)` },
      ],
      { duration: 200, delay: 90 + index * 55, easing: RX_SLAM, fill: "both" },
    );
  });
  root.querySelector(".rx-line")?.animate(
    [{ opacity: 0, transform: "translateY(8px)" }, { opacity: 1, transform: "translateY(0)" }],
    { duration: 200, delay: 280, easing: RX_EASE, fill: "both" },
  );
  const victims = [...root.querySelectorAll(".rx-victim")];
  victims.forEach((victim, index) => {
    const appear = (level === 1 ? 0 : 300) + index * 40;
    victim.animate([{ opacity: 0, transform: "translateY(10px)" }, { opacity: 1, transform: "translateY(0)" }], {
      duration: 180, delay: appear, easing: RX_EASE, fill: "both",
    });
    if (victim.classList.contains("is-new")) playShot(victim, appear + 120);
  });
}

/**
 * 最新の相手を撃ち抜く。曳光弾が飛び込み、当たった瞬間に光り、アイコンがのけぞる。
 * 弾の跡が残り、ヘッドショットなら頭蓋骨が飛び出して、最後に斜線で消す。
 */
function playShot(victim, start) {
  const headshot = victim.classList.contains("is-headshot");
  const hit = start + 90;
  victim.querySelector(".rx-tracer")?.animate(
    [
      { opacity: 1, transform: "translateX(-160px) scaleX(0.6)" },
      { opacity: 1, transform: "translateX(0) scaleX(1)", offset: 0.85 },
      { opacity: 0, transform: "translateX(6px) scaleX(0.2)" },
    ],
    { duration: 110, delay: start, easing: "linear", fill: "both" },
  );
  victim.querySelector(".rx-impact")?.animate(
    [{ opacity: 1, transform: "scale(0.35)" }, { opacity: 0, transform: "scale(1.7)" }],
    { duration: 260, delay: hit, easing: RX_EASE, fill: "both" },
  );
  victim.querySelector(".rx-face-icon")?.animate(
    [
      { transform: "translate(0, 0) rotate(0)" },
      { transform: `translate(6px, ${headshot ? -5 : -2}px) rotate(${headshot ? 14 : 7}deg)`, offset: 0.25 },
      { transform: "translate(0, 0) rotate(0)" },
    ],
    { duration: 300, delay: hit, easing: RX_EASE },
  );
  victim.querySelector(".rx-hole")?.animate([{ opacity: 0, transform: "scale(1.8)" }, { opacity: 1, transform: "scale(1)" }], {
    duration: 140, delay: hit, easing: RX_EASE, fill: "both",
  });
  victim.querySelector(".rx-skull")?.animate(
    [
      { opacity: 0, transform: "translateY(10px) scale(0.6) rotate(-14deg)" },
      { opacity: 1, transform: "translateY(-4px) scale(1.15) rotate(6deg)", offset: 0.6 },
      { opacity: 1, transform: "translateY(0) scale(1) rotate(0)" },
    ],
    { duration: 300, delay: hit + 40, easing: RX_SLAM, fill: "both" },
  );
  victim.querySelector(".rx-strike")?.animate(
    [{ transform: "rotate(-30deg) scaleX(0.05)" }, { transform: "rotate(-30deg) scaleX(1)" }],
    { duration: 180, delay: hit + (headshot ? 260 : 160), easing: RX_EASE, fill: "both" },
  );
}

/** 出は入りより短く、帯ごと右へ抜ける。 */
function playReactionOut(root) {
  const target = root.querySelector(".rx-band, .rx-log") ?? root;
  const animation = reducedMotion()
    ? root.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 120, easing: "ease-out", fill: "both" })
    : target.animate([{ clipPath: "inset(0 0 0 0)" }, { clipPath: "inset(0 0 0 100%)" }], { duration: 160, easing: RX_EASE, fill: "both" });
  animation.finished.then(() => root.remove()).catch(() => root.remove());
}

/** 演出を 1 回出す。前の演出が残っていれば差し替える。 */
async function showReaction(streak) {
  const layer = reactionLayer();
  if (!layer) return;
  const level = reactionLevel(streak.count);
  const halftone = level >= 2 ? await reactionHalftone() : null;
  const root = reactionNode(streak, halftone);
  layer.replaceChildren(root);
  playReactionIn(root, level);
  if (level >= 3 && !reducedMotion()) {
    const video = layer.parentElement?.querySelector("video");
    const amplitude = level >= 5 ? 7 : 4;
    video?.animate(
      [0, -1, 1, -0.6, 0.6, 0].map((step) => ({ transform: `translate(${step * amplitude}px, ${-step * amplitude * 0.5}px)` })),
      { duration: 300, easing: "cubic-bezier(0.36, 0.07, 0.19, 0.97)" },
    );
  }
  clearTimeout(reaction.timer);
  reaction.timer = setTimeout(() => playReactionOut(root), REACTION_HOLD_MS[level]);
}

/**
 * 再生中の動画を見張り、自分のキルの瞬間に演出を出す。`kills` は録画の先頭からのミリ秒と、
 * 倒した相手、武器、ラウンド。timeupdate は間隔が粗いので、再生中は毎フレーム確かめる。
 */
function watchReactions(video, kills) {
  cancelAnimationFrame(reaction.frame);
  const sorted = [...kills].sort((a, b) => a.at_ms - b.at_ms);
  reaction.shown = new Set();
  let previous = video.currentTime * 1000;
  const tick = () => {
    const now = video.currentTime * 1000;
    // 巻き戻したら、その先の演出をもう一度出せるようにする。
    if (now < previous - 250) reaction.shown = new Set(sorted.filter((kill) => kill.at_ms < now).map((kill) => kill.at_ms));
    sorted.forEach((kill, index) => {
      if (!reaction.shown.has(kill.at_ms) && previous <= kill.at_ms && now >= kill.at_ms) {
        reaction.shown.add(kill.at_ms);
        showReaction(streakAt(sorted, index));
      }
    });
    previous = now;
    if (!video.paused && !video.ended) reaction.frame = requestAnimationFrame(tick);
  };
  video.onplay = () => {
    previous = video.currentTime * 1000;
    cancelAnimationFrame(reaction.frame);
    reaction.frame = requestAnimationFrame(tick);
  };
  if (!video.paused) video.onplay();
}

// 書き出し。動画の重ね合わせは静止画しか扱えず、透明が効くかも確かめていないので、
// 帯は四角い不透明な板にし、版を 1 枚ずつ足した画像を短く並べて刷る動きにする。

function drawPrinted(context, text, x, y, plates) {
  const colors = { yellow: SHARE_COLOR.yellow, red: SHARE_COLOR.red, paper: SHARE_COLOR.paper };
  for (const plate of plates) {
    const [dx, dy] = PLATE_REST[plate] ?? [0, 0];
    context.fillStyle = colors[plate];
    context.fillText(text, x + dx, y + dy);
  }
}

/** 書き出し用の頭蓋骨。画面の印と同じ形を描く。 */
function drawSkull(context, x, y, size) {
  context.save();
  context.translate(x, y);
  context.scale(size / 24, size / 24);
  context.fillStyle = SHARE_COLOR.red;
  context.fill(new Path2D(SKULL_PATH));
  context.fillStyle = SHARE_COLOR.ink;
  for (const [cx, cy] of [[8.6, 10.6], [15.4, 10.6]]) {
    context.beginPath();
    context.arc(cx, cy, 2.4, 0, Math.PI * 2);
    context.fill();
  }
  context.fill(new Path2D("M12 13.4l-1.4 2.4h2.8z"));
  context.restore();
}

/** 書き出し用の帯を描く。`plates` で刷る版を選ぶ。 */
async function drawReactionBand(size, streak, plates, assets) {
  const { u } = size;
  const level = reactionLevel(streak.count);
  const scale = level >= 5 ? 1.25 : level >= 3 ? 1.05 : level === 2 ? 0.8 : 0.55;
  const icon = 40 * u;
  // 上から、刷る数字、文言、補足、倒した相手の順に積む。頭蓋骨の分だけ相手の上を空ける。
  const labelSize = 170 * u * scale;
  const labelBase = 24 * u + labelSize * 0.82;
  const messageBase = labelBase + 42 * u;
  const lineBase = messageBase + 28 * u;
  const iconTop = lineBase + 42 * u;
  const height = Math.round(iconTop + icon + 20 * u);
  const portrait = assets.halftone && level >= 2 ? height : 0;
  const label = REACTION_LABEL[level];
  const message = killLine(streak.current);
  const measure = clipCanvas(10, 10)[1];
  italic(measure, labelSize);
  const labelWidth = measure.measureText(label).width;
  measure.font = `700 ${Math.round(28 * u)}px ${TITLE_FONT}`;
  const textWidth = Math.max(labelWidth, measure.measureText(message).width, 5 * (icon + 14 * u));
  const width = Math.round(portrait + textWidth + 90 * u);
  const [canvas, context] = clipCanvas(width, height);
  context.fillStyle = level >= 5 ? SHARE_COLOR.red : SHARE_COLOR.ink;
  context.fillRect(0, 0, width, height);
  context.fillStyle = level >= 5 ? "rgb(11 11 12 / 28%)" : "rgb(242 241 236 / 10%)";
  for (let y = 3; y < height; y += 7 * u) for (let x = 3; x < width; x += 7 * u) context.fillRect(x, y, 1.6 * u, 1.6 * u);
  if (assets.halftone) context.drawImage(assets.halftone, 0, 0, portrait, portrait);
  const left = portrait + 36 * u;
  italic(context, labelSize);
  drawPrinted(context, label, left, labelBase, plates);
  context.font = `700 ${Math.round(28 * u)}px ${TITLE_FONT}`;
  context.fillStyle = SHARE_COLOR.paper;
  context.fillText(message, left, messageBase);
  mono(context, 18 * u);
  context.fillStyle = level >= 5 ? SHARE_COLOR.ink : SHARE_COLOR.mute;
  context.fillText(streakLine(streak), left, lineBase);
  streak.victims.forEach((victim, index) => {
    const x = left + index * (icon + 14 * u);
    const y = iconTop;
    const image = assets.victims.get(victim.agent);
    if (image) context.drawImage(image, x, y, icon, icon);
    else {
      context.fillStyle = SHARE_COLOR.mute;
      context.fillRect(x, y, icon, icon);
    }
    // 弾の跡。ヘッドショットは頭の位置、それ以外は胴の位置。
    const holeY = y + icon * (victim.headshot ? 0.3 : 0.62);
    context.fillStyle = SHARE_COLOR.paper;
    context.beginPath();
    context.arc(x + icon * 0.52, holeY, 5 * u, 0, Math.PI * 2);
    context.fill();
    context.fillStyle = SHARE_COLOR.ink;
    context.beginPath();
    context.arc(x + icon * 0.52, holeY, 3 * u, 0, Math.PI * 2);
    context.fill();
    if (victim.headshot) drawSkull(context, x + icon * 0.55, y - 26 * u, 26 * u);
    // 斜線はアイコンの中心を通す。
    context.strokeStyle = level >= 5 ? SHARE_COLOR.ink : SHARE_COLOR.red;
    context.lineWidth = 5 * u;
    context.beginPath();
    context.moveTo(x - 2 * u, y + icon * 0.82);
    context.lineTo(x + icon + 2 * u, y + icon * 0.18);
    context.stroke();
  });
  return canvas;
}

function reactionPlace(size, card, level) {
  if (level === 1) return [size.W * 0.03, size.H * 0.9 - card.height];
  if (level === 2) return [size.W * 0.06, size.H * 0.5 - card.height / 2];
  return [size.W / 2 - card.width / 2, size.H * 0.46 - card.height / 2];
}

/** キルごとの演出を、書き出しの層として返す。1 キルは左下の小さなキルログにする。 */
async function reactionLayers(size, clip, layerOf) {
  const kills = clip.events
    .filter((event) => event.kind === "kill")
    .map((event) => ({ at_ms: event.at_ms, victim: event.victim, weapon: event.weapon, round: clip.round, headshot: event.headshot }));
  const length = clip.end_ms - clip.start_ms;
  const victimNames = [...new Set(kills.map((kill) => kill.victim).filter(Boolean))];
  const [halftoneData, ...icons] = await Promise.all([
    reactionHalftone(),
    ...victimNames.map((victim) => loadImage(images.agents[victim]?.icon)),
  ]);
  const assets = {
    halftone: await loadDataImage(halftoneData),
    victims: new Map(victimNames.map((victim, index) => [victim, icons[index]])),
  };
  const build = [[["yellow"], 50], [["yellow", "red"], 50], [["yellow", "red", "paper"], 0]];
  const layers = [];
  for (let index = 0; index < kills.length; index += 1) {
    const streak = streakAt(kills, index);
    const level = reactionLevel(streak.count);
    let at = streak.current.at_ms;
    let placed = null;
    for (const [plates, duration] of build) {
      if (at >= length) break;
      const band = await drawReactionBand(size, streak, plates, assets);
      placed ??= reactionPlace(size, band, level);
      const hold = duration || Math.min(REACTION_HOLD_MS[level], length - at);
      layers.push(layerOf(band, placed[0], placed[1], at, Math.min(hold, length - at)));
      at += hold;
    }
  }
  return layers;
}

/** 設定画面の画像選び。 */
function setupReactionSetting() {
  const input = document.getElementById("reaction-file");
  const preview = document.getElementById("reaction-preview");
  const clear = document.getElementById("reaction-clear");
  const status = document.getElementById("reaction-status");
  if (!input) return;
  const render = async () => {
    const source = readReactionImage();
    preview.hidden = !source;
    if (source) preview.src = (await reactionHalftone()) ?? source;
    clear.disabled = !source;
  };
  input.addEventListener("change", async () => {
    const file = input.files?.[0];
    if (!file) return;
    try {
      const data = await shrinkImage(file);
      localStorage.setItem(REACTION_IMAGE_KEY, data);
      reaction.image = data;
      status.textContent = "網点に刷り直して、2K 以上の演出に使います。";
    } catch (error) {
      status.textContent = `使えませんでした: ${error.message ?? error}`;
    }
    input.value = "";
    render();
  });
  clear.addEventListener("click", () => {
    try {
      localStorage.removeItem(REACTION_IMAGE_KEY);
    } catch {
      // 消せなくても、この起動の間は画像なしで動く。
    }
    reaction.image = null;
    reaction.halftone = null;
    status.textContent = "画像なしの演出に戻しました。";
    render();
  });
  document.getElementById("reaction-test")?.addEventListener("click", () => {
    showView("clips");
    const victims = ["Jett", "Omen", "Sova", "Reyna", "Sage"];
    const kills = victims.map((victim, index) => ({ at_ms: index * 900, victim, weapon: "Vandal", round: 7, headshot: index % 2 === 0 }));
    kills.forEach((_, index) => setTimeout(() => showReaction(streakAt(kills, index)), 200 + index * 900));
  });
  render();
}

setupReactionSetting();
