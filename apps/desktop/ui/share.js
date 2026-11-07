const SHARE_SIZE = { width: 1600, height: 900 };
const SHARE_MARGIN = 72;
const SHARE_DOT = 9;
const SHARE_COLOR = { ink: "#0b0b0c", paper: "#f2f1ec", red: "#ff2a1a", yellow: "#ffe100", mute: "#a3a29d" };
const DISPLAY_FONT = "italic 900";
const MONO_FONT = '"JetBrains Mono", Consolas, monospace';
const TITLE_FONT = 'Archivo, Bahnschrift, "Yu Gothic UI", sans-serif';

/** キャンバスへ描くため、CORS を許可して読み込む。失敗しても画像なしで続ける。 */
function loadImage(url) {
  return new Promise((resolve) => {
    if (!url) {
      resolve(null);
      return;
    }
    const image = new Image();
    image.crossOrigin = "anonymous";
    image.onload = () => resolve(image);
    image.onerror = () => resolve(null);
    // 画面表示で読んだ画像のキャッシュには CORS の許可がないので、別の URL として取得する。
    image.src = `${url}?for=canvas`;
  });
}

function drawCover(context, image, width, height) {
  const scale = Math.max(width / image.width, height / image.height);
  const drawn = { width: image.width * scale, height: image.height * scale };
  context.drawImage(image, (width - drawn.width) / 2, (height - drawn.height) / 2, drawn.width, drawn.height);
}

function drawBackground(context, splash) {
  const { width, height } = SHARE_SIZE;
  context.fillStyle = SHARE_COLOR.ink;
  context.fillRect(0, 0, width, height);
  if (!splash) return;

  context.filter = "grayscale(1) contrast(1.25) brightness(0.8)";
  drawCover(context, splash, width, height);
  context.filter = "none";

  const scrim = context.createLinearGradient(0, 0, width, 0);
  scrim.addColorStop(0, "rgb(11 11 12 / 92%)");
  scrim.addColorStop(0.6, "rgb(11 11 12 / 62%)");
  scrim.addColorStop(1, "rgb(11 11 12 / 25%)");
  context.fillStyle = scrim;
  context.fillRect(0, 0, width, height);

  context.fillStyle = SHARE_COLOR.ink;
  for (let y = 0; y < height; y += SHARE_DOT) {
    for (let x = 0; x < width; x += SHARE_DOT) {
      context.beginPath();
      context.arc(x, y, SHARE_DOT * 0.28, 0, Math.PI * 2);
      context.fill();
    }
  }
}

function drawDisc(context, portrait, rank) {
  const center = { x: 1290, y: 340 };
  const radius = 240;
  context.fillStyle = SHARE_COLOR.yellow;
  context.beginPath();
  context.arc(center.x - 14, center.y - 12, radius, 0, Math.PI * 2);
  context.fill();
  context.fillStyle = SHARE_COLOR.red;
  context.beginPath();
  context.arc(center.x + 16, center.y + 12, radius, 0, Math.PI * 2);
  context.fill();

  context.save();
  context.beginPath();
  context.arc(center.x, center.y, radius, 0, Math.PI * 2);
  context.fillStyle = SHARE_COLOR.ink;
  context.fill();
  context.clip();
  if (portrait) {
    const size = radius * 2.9;
    context.drawImage(portrait, center.x - size / 2, center.y - radius * 1.05, size, size);
  }
  context.restore();
  if (rank) context.drawImage(rank, center.x + 118, center.y + 118, 150, 150);
}

function drawHeadline(context, text, x, y, size, color) {
  context.font = `${DISPLAY_FONT} ${size}px ${TITLE_FONT}`;
  context.fillStyle = color;
  context.fillText(text, x, y);
  const end = x + context.measureText(text).width;
  context.fillStyle = SHARE_COLOR.red;
  context.fillRect(end + size * 0.06, y - size * 0.22, size * 0.2, size * 0.2);
}

function drawLabel(context, text, x, y, size, color = SHARE_COLOR.paper) {
  context.font = `400 ${size}px ${MONO_FONT}`;
  context.fillStyle = color;
  context.fillText(text, x, y);
}

function drawFigures(context, review) {
  const own = review.own;
  const figures = [
    [own.acs, "ACS"],
    [own.adr, "ADR"],
    [`${own.kills}/${own.deaths}/${own.assists}`, "K/D/A"],
    [own.headshot_percent === null ? "－" : `${own.headshot_percent}%`, "HS"],
  ];
  let x = SHARE_MARGIN;
  for (const [value, label] of figures) {
    context.font = `${DISPLAY_FONT} 58px ${TITLE_FONT}`;
    context.fillStyle = SHARE_COLOR.paper;
    context.fillText(String(value), x, 660);
    const width = context.measureText(String(value)).width;
    drawLabel(context, label, x, 696, 22);
    x += Math.max(width, 96) + 56;
  }
}

function drawScene(context, scene) {
  if (!scene) return;
  const y = 750;
  context.fillStyle = SHARE_COLOR.paper;
  context.fillRect(SHARE_MARGIN, y, 620, 96);
  context.font = `${DISPLAY_FONT} 46px ${TITLE_FONT}`;
  context.fillStyle = SHARE_COLOR.ink;
  context.fillText(scene.title.toUpperCase(), SHARE_MARGIN + 20, y + 58);
  drawLabel(context, scene.detail, SHARE_MARGIN + 20, y + 84, 20, SHARE_COLOR.ink);
}

async function drawShare(canvas, review, withName) {
  await Promise.all([
    document.fonts.load(`${DISPLAY_FONT} 100px Archivo`),
    document.fonts.load(`400 20px "JetBrains Mono"`),
  ]).catch(() => {});

  const agent = review.own.agent;
  const [splash, portrait, rank] = await Promise.all([
    loadImage(images.maps[review.map]?.splash),
    loadImage(images.agents[agent]?.portrait),
    loadImage(images.tiers[review.own.tier]?.large),
  ]);

  canvas.width = SHARE_SIZE.width;
  canvas.height = SHARE_SIZE.height;
  const context = canvas.getContext("2d");
  drawBackground(context, splash);
  drawDisc(context, portrait, rank);

  const outcome = review.won === null ? "DRAW" : review.won ? "WIN" : "LOSS";
  const color = review.won === false ? SHARE_COLOR.red : SHARE_COLOR.paper;
  const name = withName ? app.profile?.name : null;
  const label = [name, review.map, agent, review.own.rank].filter(Boolean).join(" / ");
  drawLabel(context, label, SHARE_MARGIN, 120, 26);
  drawHeadline(context, outcome, SHARE_MARGIN, 300, 190, color);
  drawHeadline(context, `${review.own_score}-${review.enemy_score}`, SHARE_MARGIN, 490, 170, SHARE_COLOR.paper);
  drawFigures(context, review);
  drawScene(context, review.highlights?.[0]);
  drawLabel(context, "vRY", SHARE_SIZE.width - 130, SHARE_SIZE.height - 50, 30, SHARE_COLOR.mute);
}

function canvasBlob(canvas) {
  return new Promise((resolve) => { canvas.toBlob(resolve, "image/png"); });
}

async function saveShare(canvas, status) {
  try {
    const data = canvas.toDataURL("image/png").split(",")[1];
    const path = await invoke("save_share_image", { pngBase64: data });
    status.textContent = `保存しました: ${path}`;
  } catch (error) {
    status.textContent = `保存できませんでした: ${error}`;
  }
}

async function copyShare(canvas, status) {
  try {
    const blob = await canvasBlob(canvas);
    await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
    status.textContent = "画像をコピーしました";
  } catch (error) {
    status.textContent = `コピーできませんでした: ${error}`;
  }
}

async function openShare(review) {
  const panel = document.getElementById("share-panel");
  const canvas = element("canvas", "share-canvas");
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", "試合結果の共有画像");

  const status = element("span");
  status.setAttribute("role", "status");
  const nameLabel = element("label");
  const nameToggle = element("input");
  nameToggle.type = "checkbox";
  nameLabel.append(nameToggle, "自分の名前を入れる");
  nameToggle.addEventListener("change", () => drawShare(canvas, review, nameToggle.checked));

  const save = element("button", "primary", "画像を保存");
  save.type = "button";
  save.addEventListener("click", () => saveShare(canvas, status));
  const copy = element("button", "tag", "コピー");
  copy.type = "button";
  copy.addEventListener("click", () => copyShare(canvas, status));

  const actions = element("div", "share-actions");
  actions.append(save, copy, nameLabel, status);
  panel.replaceChildren(canvas, actions);
  await drawShare(canvas, review, false);
}
