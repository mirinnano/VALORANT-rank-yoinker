// 共有用クリップのオーバーレイ。プロの大会中継のように、選手プレートとキルフィードを重ねる。
// 透過に頼らず、板ごとに塗りつぶした画像を作って、Windows 側で決まった位置に重ねる。

const CLIP_INTRO_MS = 1100;
const CLIP_STAMP_MS = 1600;
const CLIP_NAME_KEY = "clip-share-name";

function clipCanvas(width, height) {
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(width);
  canvas.height = Math.round(height);
  return [canvas, canvas.getContext("2d")];
}

function pngOf(canvas) {
  return canvas.toDataURL("image/png").split(",")[1];
}

function italic(context, size) {
  context.font = `${DISPLAY_FONT} ${size}px ${TITLE_FONT}`;
}

function mono(context, size) {
  context.font = `400 ${size}px ${MONO_FONT}`;
}

/** 赤い四角のピリオド。ブランドの印。 */
function period(context, x, y, size) {
  context.fillStyle = SHARE_COLOR.red;
  context.fillRect(x + size * 0.06, y - size * 0.22, size * 0.2, size * 0.2);
}

/** 文字が幅に収まるまで小さくする。 */
function fitText(context, text, maxWidth, size, setFont) {
  let current = size;
  setFont(context, current);
  while (current > 10 && context.measureText(text).width > maxWidth) {
    current -= 2;
    setFont(context, current);
  }
  return current;
}

function drawImageContain(context, image, x, y, size) {
  if (!image) return;
  const scale = Math.min(size / image.width, size / image.height);
  const w = image.width * scale;
  const h = image.height * scale;
  context.drawImage(image, x + (size - w) / 2, y + (size - h) / 2, w, h);
}

function halftone(context, width, height, dot) {
  context.fillStyle = SHARE_COLOR.ink;
  for (let y = 0; y < height; y += dot) {
    for (let x = 0; x < width; x += dot) {
      context.beginPath();
      context.arc(x, y, dot * 0.28, 0, Math.PI * 2);
      context.fill();
    }
  }
}

/** 冒頭のタイトルカード。画面全体。 */
function drawIntro(size, scene, assets) {
  const { W, H, u } = size;
  const [canvas, context] = clipCanvas(W, H);
  context.fillStyle = SHARE_COLOR.ink;
  context.fillRect(0, 0, W, H);
  if (assets.splash) {
    context.filter = "grayscale(1) contrast(1.3) brightness(0.55)";
    drawCover(context, assets.splash, W, H);
    context.filter = "none";
    halftone(context, W, H, 10 * u);
  }
  // 右に選手、左に見出し。
  if (assets.portrait) {
    const height = H * 1.15;
    const width = (assets.portrait.width / assets.portrait.height) * height;
    context.drawImage(assets.portrait, W - width * 0.82, H * 0.02, width, height);
  }
  context.fillStyle = SHARE_COLOR.red;
  context.fillRect(0, 0, 18 * u, H);

  mono(context, 30 * u);
  context.fillStyle = SHARE_COLOR.paper;
  context.fillText(scene.kicker, 96 * u, 130 * u);

  const title = scene.title.toUpperCase();
  const titleSize = fitText(context, title, W * 0.62, 340 * u, italic);
  context.fillStyle = scene.accent;
  context.fillText(title, 84 * u, H * 0.6);
  period(context, 84 * u + context.measureText(title).width, H * 0.6, titleSize);

  italic(context, 72 * u);
  context.fillStyle = SHARE_COLOR.paper;
  context.fillText(scene.player.toUpperCase(), 96 * u, H * 0.6 + 110 * u);
  mono(context, 30 * u);
  context.fillStyle = SHARE_COLOR.mute;
  context.fillText(scene.detail, 96 * u, H * 0.6 + 164 * u);

  mono(context, 28 * u);
  context.fillStyle = SHARE_COLOR.paper;
  const brand = "busters.gg";
  context.fillText(brand, W - context.measureText(brand).width - 64 * u, H - 56 * u);
  return canvas;
}

/** 左下の選手プレート。上にラウンドとスコアのタブを付ける。 */
function drawPlayerPlate(size, scene, assets) {
  const { u } = size;
  const width = 660 * u;
  const tab = 54 * u;
  const body = 150 * u;
  const [canvas, context] = clipCanvas(width, tab + body);

  // タブ: ラウンド、マップ、スコア。
  context.fillStyle = SHARE_COLOR.ink;
  context.fillRect(0, 0, width, tab);
  let x = 0;
  if (scene.round) {
    context.fillStyle = SHARE_COLOR.red;
    context.fillRect(0, 0, 96 * u, tab);
    italic(context, 34 * u);
    context.fillStyle = SHARE_COLOR.paper;
    context.fillText(`R${scene.round}`, 16 * u, tab - 14 * u);
    x = 96 * u;
  }
  italic(context, 30 * u);
  context.fillStyle = SHARE_COLOR.paper;
  context.fillText((scene.map ?? "").toUpperCase(), x + 18 * u, tab - 15 * u);
  if (scene.score) {
    italic(context, 32 * u);
    context.fillStyle = SHARE_COLOR.yellow;
    const score = `${scene.score[0]}-${scene.score[1]}`;
    context.fillText(score, width - context.measureText(score).width - 18 * u, tab - 14 * u);
  }

  // 本体: エージェント、名前、ランク、試合の数字。
  context.fillStyle = SHARE_COLOR.yellow;
  context.fillRect(0, tab, width, 6 * u);
  context.fillStyle = "#151517";
  context.fillRect(0, tab + 6 * u, width, body - 6 * u);
  context.fillStyle = SHARE_COLOR.red;
  context.fillRect(0, tab + 6 * u, body - 6 * u, body - 6 * u);
  drawImageContain(context, assets.icon, 10 * u, tab + 16 * u, body - 26 * u);

  const left = body + 14 * u;
  const nameSize = fitText(context, scene.player.toUpperCase(), width - left - 150 * u, 50 * u, italic);
  context.fillStyle = SHARE_COLOR.paper;
  context.fillText(scene.player.toUpperCase(), left, tab + 6 * u + 58 * u);
  const nameEnd = left + context.measureText(scene.player.toUpperCase()).width;
  period(context, nameEnd, tab + 6 * u + 58 * u, nameSize);
  if (assets.rank) context.drawImage(assets.rank, width - 120 * u, tab + 14 * u, 50 * u, 50 * u);
  mono(context, 20 * u);
  context.fillStyle = SHARE_COLOR.mute;
  if (scene.rank) context.fillText(scene.rank, width - 120 * u - context.measureText(scene.rank).width - 8 * u, tab + 46 * u);

  let statX = left;
  for (const [label, value] of scene.stats) {
    mono(context, 18 * u);
    context.fillStyle = SHARE_COLOR.mute;
    context.fillText(label, statX, tab + 104 * u);
    italic(context, 34 * u);
    context.fillStyle = SHARE_COLOR.paper;
    context.fillText(String(value), statX, tab + 138 * u);
    statX += Math.max(context.measureText(String(value)).width, 60 * u) + 30 * u;
  }
  return canvas;
}

/** キルフィードの 1 行。番号、武器、倒した相手。 */
function drawFeedRow(size, event, count, assets) {
  const { u } = size;
  const width = 540 * u;
  const height = 68 * u;
  const [canvas, context] = clipCanvas(width, height);
  context.fillStyle = SHARE_COLOR.ink;
  context.fillRect(0, 0, width, height);
  context.fillStyle = event.kind === "defuse" ? SHARE_COLOR.red : SHARE_COLOR.yellow;
  context.fillRect(0, 0, height, height);
  italic(context, 40 * u);
  context.fillStyle = event.kind === "defuse" ? SHARE_COLOR.paper : SHARE_COLOR.ink;
  const mark = event.kind === "defuse" ? "D" : String(count);
  context.fillText(mark, (height - context.measureText(mark).width) / 2, height - 18 * u);

  const left = height + 18 * u;
  if (event.kind === "defuse") {
    italic(context, 34 * u);
    context.fillStyle = SHARE_COLOR.paper;
    context.fillText("SPIKE DEFUSED", left, height - 20 * u);
    return canvas;
  }
  italic(context, 30 * u);
  context.fillStyle = SHARE_COLOR.paper;
  const weapon = (event.weapon ?? "KILL").toUpperCase();
  context.fillText(weapon, left, height - 22 * u);
  // 武器から相手へ向かう矢印。線で描く。
  const arrowX = left + context.measureText(weapon).width + 18 * u;
  context.strokeStyle = SHARE_COLOR.red;
  context.lineWidth = 5 * u;
  context.beginPath();
  context.moveTo(arrowX, height / 2 - 10 * u);
  context.lineTo(arrowX + 12 * u, height / 2);
  context.lineTo(arrowX, height / 2 + 10 * u);
  context.stroke();
  const victimX = arrowX + 30 * u;
  drawImageContain(context, assets.victims.get(event.victim), victimX, 9 * u, 50 * u);
  mono(context, 22 * u);
  context.fillStyle = SHARE_COLOR.mute;
  context.fillText(event.victim ?? "", victimX + 60 * u, height - 25 * u);
  return canvas;
}

/** 最後のキルで出す大きなスタンプ。 */
function drawStamp(size, text, ace) {
  const { u } = size;
  const width = 520 * u;
  const height = 200 * u;
  const [canvas, context] = clipCanvas(width, height);
  context.fillStyle = ace ? SHARE_COLOR.red : SHARE_COLOR.yellow;
  context.fillRect(0, 0, width, height);
  context.fillStyle = SHARE_COLOR.ink;
  context.fillRect(0, height - 14 * u, width, 14 * u);
  const label = text.toUpperCase();
  const fontSize = fitText(context, label, width - 90 * u, 150 * u, italic);
  context.fillStyle = ace ? SHARE_COLOR.paper : SHARE_COLOR.ink;
  context.fillText(label, 36 * u, height - 46 * u);
  period(context, 36 * u + context.measureText(label).width, height - 46 * u, fontSize);
  return canvas;
}

function scoreBefore(review, round) {
  if (!review || !round) return null;
  const before = review.rounds.filter((entry) => entry.number < round);
  return [before.filter((entry) => entry.won).length, before.filter((entry) => !entry.won).length];
}

/** オーバーレイの材料を集める。 */
async function clipScene(recording, clip, withName) {
  const review = recording.match_id
    ? await invoke("load_review", { matchId: recording.match_id }).catch(() => null)
    : null;
  const own = review?.own;
  const agent = own?.agent ?? null;
  const name = withName ? app.profile?.name?.split("#")[0] : null;
  const accent = clip.kind === "clutch" || clip.kills >= 5 ? SHARE_COLOR.red : SHARE_COLOR.paper;
  const stats = own
    ? [["ACS", own.acs], ["K/D/A", `${own.kills}/${own.deaths}/${own.assists}`], ["HS", own.headshot_percent === null ? "－" : `${own.headshot_percent}%`], ["ADR", own.adr]]
    : [];
  return {
    title: clip.title === "キル" ? "Kill" : clip.title === "保存した場面" ? "Play" : clip.title,
    accent,
    kicker: "busters.gg / CLIP",
    player: name ?? agent ?? "Player",
    detail: [clip.round ? `ROUND ${clip.round}` : null, recording.map, recording.mode, agent].filter(Boolean).join(" / ").toUpperCase(),
    map: recording.map,
    round: clip.round,
    score: scoreBefore(review, clip.round),
    rank: own?.rank ?? null,
    tier: own?.tier ?? null,
    agent,
    stats,
  };
}

/** 書き出しに渡す画像と位置を作る。 */
async function buildClipOverlay(recording, clip, withName) {
  await Promise.all([
    document.fonts.load(`${DISPLAY_FONT} 100px Archivo`),
    document.fonts.load(`400 20px "JetBrains Mono"`),
  ]);
  const W = recording.width || 1920;
  const H = recording.height || 1080;
  const size = { W, H, u: H / 1080 };
  const scene = await clipScene(recording, clip, withName);
  const victimNames = [...new Set(clip.events.map((event) => event.victim).filter(Boolean))];
  const [splash, portrait, icon, rank, ...victimIcons] = await Promise.all([
    loadImage(images.maps[scene.map]?.splash),
    loadImage(images.agents[scene.agent]?.portrait),
    loadImage(images.agents[scene.agent]?.icon),
    loadImage(images.tiers[scene.tier]?.small),
    ...victimNames.map((victim) => loadImage(images.agents[victim]?.icon)),
  ]);
  const assets = { splash, portrait, icon, rank, victims: new Map(victimNames.map((v, i) => [v, victimIcons[i]])) };
  const length = clip.end_ms - clip.start_ms;
  const layer = (canvas, x, y, delay, duration) => ({
    png_base64: pngOf(canvas), x: x / W, y: y / H, width: canvas.width / W, height: canvas.height / H,
    delay_ms: Math.max(0, Math.round(delay)), duration_ms: Math.max(1, Math.round(duration)),
  });

  const layers = [];
  const plate = drawPlayerPlate(size, scene, assets);
  layers.push(layer(plate, 32 * size.u, H - plate.height - 36 * size.u, 0, length));

  let kills = 0;
  clip.events.forEach((event, index) => {
    if (event.kind === "kill") kills += 1;
    const row = drawFeedRow(size, event, kills, assets);
    layers.push(layer(row, W - row.width - 32 * size.u, H * 0.3 + index * 78 * size.u, event.at_ms, length - event.at_ms));
  });
  const last = clip.events.at(-1);
  if (last && (clip.kills >= 2 || clip.kind === "clutch")) {
    const stamp = drawStamp(size, scene.title, clip.kills >= 5);
    layers.push(layer(stamp, 48 * size.u, H * 0.34, last.at_ms + 150, Math.min(CLIP_STAMP_MS, length - last.at_ms)));
  }
  const intro = { png_base64: pngOf(drawIntro(size, scene, assets)), duration_ms: CLIP_INTRO_MS };
  return { intro, layers, preview: { size, scene, assets } };
}

function readShareName() {
  try {
    return localStorage.getItem(CLIP_NAME_KEY) !== "false";
  } catch {
    return true;
  }
}

function writeShareName(value) {
  try {
    localStorage.setItem(CLIP_NAME_KEY, String(value));
  } catch {
    // 保存できなくても、この画面では選んだ値を使う。
  }
}

/** ボタン 1 つで共有用の動画を作る。 */
async function exportShareClip(recording, clip, button, meta) {
  button.disabled = true;
  const label = button.textContent;
  button.textContent = "作っています…";
  try {
    const { intro, layers } = await buildClipOverlay(recording, clip, readShareName());
    const path = await invoke("export_share_clip", { clipId: clip.id, intro, layers });
    meta.replaceChildren(`保存先: ${path} `);
    const reveal = element("button", "tag", "フォルダで表示");
    reveal.type = "button";
    reveal.addEventListener("click", () => invoke("reveal_file", { path }));
    meta.append(reveal);
    button.textContent = "もう一度作る";
  } catch (error) {
    meta.textContent = `作れませんでした: ${error}`;
    button.textContent = label;
  } finally {
    button.disabled = false;
  }
}
