// パーティ募集の画像。share.js の描画部品を使い、ランクと空き枠、参加リンクを1枚にする。
// 他のメンバーの名前は載せない。招待文と同じく、ランクだけを出す。
const RECRUIT_SLOT = { size: 150, gap: 28, top: 470 };

function recruitMembers(snapshot, profile) {
  const lobby = snapshot.lobby;
  const max = lobby?.max_party_size || FULL_PARTY;
  const members = (lobby?.party ?? []).map((member) => ({
    rank: member.is_own ? profile?.rank ?? member.rank : member.rank,
    owner: member.owner,
  }));
  const size = Math.max(lobby?.party_size ?? 1, members.length);
  while (members.length < size) members.push({ rank: null, owner: false });
  return { members, max, open: Math.max(max - size, 0) };
}

function drawSlot(context, x, slot, icon) {
  const { size, top } = RECRUIT_SLOT;
  if (!slot) {
    context.setLineDash([10, 8]);
    context.strokeStyle = SHARE_COLOR.mute;
    context.lineWidth = 3;
    context.strokeRect(x, top, size, size);
    context.setLineDash([]);
    drawLabel(context, "OPEN", x + size / 2 - 30, top + size / 2 + 8, 24, SHARE_COLOR.mute);
    return;
  }
  context.fillStyle = SHARE_COLOR.ink;
  context.fillRect(x, top, size, size);
  if (icon) context.drawImage(icon, x + 15, top + 10, size - 30, size - 30);
  const label = slot.rank?.tier > 0 ? slot.rank.name : "ランクなし";
  drawLabel(context, label, x, top + size + 34, 22);
  if (slot.owner) drawLabel(context, "LEADER", x, top + size + 62, 18, SHARE_COLOR.yellow);
}

function drawJoinPanel(context, code, link) {
  const x = SHARE_MARGIN;
  const y = 760;
  const width = SHARE_SIZE.width - SHARE_MARGIN * 2;
  context.fillStyle = SHARE_COLOR.paper;
  context.fillRect(x, y, width, 92);
  context.font = `${DISPLAY_FONT} 44px ${TITLE_FONT}`;
  context.fillStyle = SHARE_COLOR.ink;
  const head = code ? `CODE ${code}` : "JOIN";
  context.fillText(head, x + 24, y + 60);
  if (link) {
    const offset = x + 48 + context.measureText(head).width;
    drawLabel(context, link, offset, y + 58, 24, SHARE_COLOR.ink);
  }
}

async function drawRecruit(canvas, snapshot, profile, code, link) {
  await Promise.all([
    document.fonts.load(`${DISPLAY_FONT} 100px Archivo`),
    document.fonts.load(`400 20px "JetBrains Mono"`),
  ]).catch(() => {});
  const { members, max, open } = recruitMembers(snapshot, profile);
  const own = snapshot.lobby?.party?.find((member) => member.is_own);
  const [background, ...icons] = await Promise.all([
    loadImage(own?.card_wide ?? images.maps[profile?.history.at(-1)?.map]?.splash),
    ...members.map((member) => (member.rank?.tier > 0 ? loadImage(images.tiers[member.rank.tier]?.large) : null)),
  ]);

  canvas.width = SHARE_SIZE.width;
  canvas.height = SHARE_SIZE.height;
  const context = canvas.getContext("2d");
  drawBackground(context, background);
  drawLabel(context, ["VALORANT", snapshot.mode ?? "PARTY"].join(" / "), SHARE_MARGIN, 120, 26);
  drawHeadline(context, "PARTY UP", SHARE_MARGIN, 290, 170, SHARE_COLOR.paper);
  const need = open > 0 ? `あと${open}人 募集中` : "満員";
  drawLabel(context, `${need}　${members.length}/${max}`, SHARE_MARGIN, 380, 40, open > 0 ? SHARE_COLOR.yellow : SHARE_COLOR.mute);

  for (let index = 0; index < max; index += 1) {
    const x = SHARE_MARGIN + index * (RECRUIT_SLOT.size + RECRUIT_SLOT.gap);
    drawSlot(context, x, members[index] ?? null, icons[index] ?? null);
  }
  drawJoinPanel(context, code, link);
  drawLabel(context, "busters.gg", SHARE_SIZE.width - 230, 60, 26, SHARE_COLOR.mute);
}

/** 招待欄の下に画像と保存、コピーのボタンを出す。もう一度押すと閉じる。 */
async function toggleRecruit(block, button, snapshot, code, link) {
  const existing = block.querySelector(".recruit");
  if (existing) {
    existing.remove();
    lobbyView.recruitOpen = false;
    button.textContent = "募集画像を作る";
    button.setAttribute("aria-expanded", "false");
    return;
  }
  lobbyView.recruitOpen = true;
  button.textContent = "募集画像を閉じる";
  button.setAttribute("aria-expanded", "true");
  const box = element("div", "recruit");
  const canvas = element("canvas", "share-canvas");
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", "パーティ募集の画像。ランク、空き枠、参加方法を載せています");
  const status = element("span");
  status.setAttribute("role", "status");
  const copy = element("button", "primary", "画像をコピー");
  copy.type = "button";
  copy.addEventListener("click", () => copyShare(canvas, status));
  const save = element("button", "tag", "画像を保存");
  save.type = "button";
  save.addEventListener("click", () => saveShare(canvas, status));
  const actions = element("div", "share-actions");
  actions.append(copy, save, status);
  box.append(canvas, actions);
  block.append(box);
  await drawRecruit(canvas, snapshot, app.profile, code, link);
}
