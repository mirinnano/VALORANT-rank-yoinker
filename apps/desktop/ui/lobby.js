const LOBBY_STATUS = { ready: "READY", queueing: "QUEUEING", match_found: "MATCH FOUND", custom: "CUSTOM" };
const DAY_MS = 86_400_000;
const FULL_PARTY = 5;
const RR_PER_RANK = 100;
/** Immortal 以上は RR の上限がなく、次のランクまでの距離を出せない。 */
const LAST_LADDER_TIER = 24;
const INVITE_HOLD_MS = 20_000;
const lobbyView = { signature: "", phase: null, invite: null };

function memberBanner(member, profile) {
  const banner = element("li", member.is_own ? "party-banner is-own" : "party-banner");
  const art = picture(member.card_large, "banner-art");
  if (art) banner.append(art);
  const name = member.is_own ? profile?.name ?? member.name : member.name;
  const rank = member.is_own ? profile?.rank ?? member.rank : member.rank;
  const plate = element("div", "banner-plate");
  const flags = [member.owner ? "リーダー" : null, member.level ? `Lv ${member.level}` : null].filter(Boolean).join("、");
  if (flags) plate.append(element("small", "", flags));
  plate.append(element("b", "party-name", name ?? "名前を取得できません"));
  const title = images.titles?.[member.title_id];
  if (title) plate.append(element("span", "party-title", title));
  if (rank) {
    const line = element("span", "party-rank");
    line.append(rankIcon(rank.tier, 28), rank.tier > 0 ? rank.name : "ランクなし");
    plate.append(line);
  }
  banner.append(plate);
  return banner;
}

function unknownBanner() {
  const banner = element("li", "party-banner is-unknown");
  banner.append(element("b", "party-name", "メンバー"), element("small", "", "詳細を取得できません"));
  return banner;
}

function emptyBanner(onInvite) {
  const banner = element("li", "party-banner is-empty");
  const button = element("button", "banner-invite", "招待");
  button.type = "button";
  button.addEventListener("click", onInvite);
  banner.append(button);
  return banner;
}

function partyBanners(lobby, profile, onInvite) {
  const members = lobby?.party ?? [{ is_own: true }];
  const size = Math.max(lobby?.party_size ?? 1, members.length);
  const total = Math.max(lobby?.max_party_size || FULL_PARTY, size);
  const list = element("ol", "party-banners");
  list.append(...members.map((member) => memberBanner(member, profile)));
  for (let index = members.length; index < total; index += 1) {
    list.append(index < size ? unknownBanner() : emptyBanner(onInvite));
  }
  return list;
}

/** 発行直後は次の更新が届くまで、手元の結果を優先する。 */
function inviteCode(lobby) {
  const held = lobbyView.invite;
  if (held && Date.now() < held.until) return held.code;
  return lobby?.invite_code ?? null;
}

async function changeInvite(enabled, status) {
  status.textContent = enabled ? "招待コードを発行しています" : "招待コードを無効にしています";
  try {
    const code = await window.__TAURI__.core.invoke("set_party_invite", { enabled });
    lobbyView.invite = { code, until: Date.now() + INVITE_HOLD_MS };
    lobbyView.signature = "";
    if (app.lastEvent?.lobby) renderLobby(views.lobby, app.lastEvent, app.profile);
  } catch (error) {
    status.textContent = `操作できませんでした: ${error}`;
  }
}

async function copyText(text, status, done) {
  try {
    await navigator.clipboard.writeText(text);
    status.textContent = done;
  } catch {
    status.textContent = "コピーできませんでした。";
  }
}

/** 他のメンバーの名前は外へ出さず、ランクだけを載せる。 */
function inviteMessage(code, snapshot, profile) {
  const lobby = snapshot.lobby;
  const size = lobby?.party_size ?? 1;
  const max = lobby?.max_party_size || FULL_PARTY;
  const ranks = (lobby?.party ?? []).map((member) => {
    const rank = member.is_own ? profile?.rank ?? member.rank : member.rank;
    const label = rank?.tier > 0 ? rank.name : "ランクなし";
    return member.owner ? `${label}（リーダー）` : label;
  });
  const lines = [
    "VALORANT パーティ募集",
    `パーティコード: ${code}`,
    snapshot.mode ? `モード: ${snapshot.mode}` : null,
    `人数: ${size}/${max}（あと${Math.max(max - size, 0)}人）`,
    ranks.length > 0 ? `メンバー: ${ranks.join("、")}` : null,
    "参加方法: ソーシャルパネルのパーティコードに入力してください。同じ地域のプレイヤーだけが参加できます。",
  ];
  return lines.filter(Boolean).join("\n");
}

function actionButton(label, className, action) {
  const button = element("button", className, label);
  button.type = "button";
  button.addEventListener("click", action);
  return button;
}

function inviteBlock(snapshot) {
  const block = element("section", "lobby-block invite");
  block.append(element("h2", "", "INVITE / 招待"));
  const status = element("p", "lobby-note");
  status.setAttribute("role", "status");
  const code = inviteCode(snapshot.lobby);
  if (code) {
    const message = inviteMessage(code, snapshot, app.profile);
    block.append(
      element("p", "invite-code", code),
      element("p", "invite-message", message),
      actionButton("コードをコピー", "primary", () => copyText(code, status, "コードをコピーしました。")),
      actionButton("招待文をコピー", "tag", () => copyText(message, status, "招待文をコピーしました。")),
      actionButton("コードを無効にする", "tag", () => changeInvite(false, status)),
    );
  } else {
    block.append(
      element("p", "lobby-note", "コードを知っている人がパーティへ参加できます。同じ地域のプレイヤーだけが使えます。"),
      actionButton("招待コードを発行", "primary", () => changeInvite(true, status)),
    );
  }
  block.append(status);
  return block;
}

/** 勝敗は RR の増減で数える。増減なしは引き分けとして含めない。 */
function lastDay(history, now) {
  const games = history.filter((point) => now - point.at_ms < DAY_MS);
  return {
    games: games.length,
    wins: games.filter((point) => point.earned > 0).length,
    losses: games.filter((point) => point.earned < 0).length,
    rr: games.reduce((sum, point) => sum + point.earned, 0),
  };
}

function todayBlock(profile) {
  const block = element("section", "lobby-block");
  block.append(element("h2", "", "LAST 24H / 直近24時間"));
  const day = lastDay(profile.history, Date.now());
  if (day.games === 0) {
    block.append(element("p", "lobby-note", "この24時間のコンペティティブはまだありません。"));
    return block;
  }
  const figures = element("div", "hero-figures");
  figures.append(
    figure(`${day.wins}勝 ${day.losses}敗`, `${day.games}試合`),
    figure(formatSigned(day.rr), "RR", signClass(day.rr)),
  );
  block.append(figures);
  return block;
}

function ladderBlock(profile) {
  const { tier, rr } = profile.rank;
  if (tier === 0 || tier >= LAST_LADDER_TIER) return null;
  const block = element("section", "lobby-block");
  block.append(element("h2", "", "NEXT RANK / 次のランクまで"));
  block.append(element("p", "lobby-next", `あと ${Math.max(RR_PER_RANK - rr, 0)} RR`));
  const bar = element("div", "ladder");
  const fill = element("i");
  fill.style.transform = `scaleX(${Math.min(rr / RR_PER_RANK, 1)})`;
  bar.append(fill);
  block.append(bar);
  return block;
}

function lastMatchBlock(profile) {
  const entry = profile.matches?.[0];
  if (!entry) return null;
  const block = element("section", "lobby-block");
  block.append(element("h2", "", "LAST MATCH / 前の試合"));
  const button = element("button", "lobby-last");
  button.type = "button";
  const thumbnail = picture(images.maps[entry.map]?.thumbnail);
  if (thumbnail) button.append(thumbnail);
  const [outcome, outcomeClass] = matchOutcome(entry);
  button.append(
    element("span", "", entry.map ?? entry.mode ?? "試合"),
    element("span", outcomeClass, outcome),
    element("small", "", `${entry.mode ?? ""} 振り返りを開く`.trim()),
  );
  button.addEventListener("click", () => openReviewFromHome(entry.match_id));
  block.append(button);
  return block;
}

/** 詳細の取得は 1 試合ずつ進むので、途中であることを隠さない。 */
function syncNote(profile) {
  const total = profile.history.length;
  if (total === 0 || profile.detailed_games >= total) return null;
  return element("p", "lobby-note", `過去${total}試合のうち ${profile.detailed_games}試合の詳細を集計しました。残りは順に取得します。`);
}

function lobbySide(profile, snapshot) {
  const side = element("div", "lobby-side");
  side.append(inviteBlock(snapshot));
  if (!profile) {
    side.append(element("p", "lobby-note", "プロフィールを読み込んでいます。"));
    return side;
  }
  const repeated = element("div");
  side.append(...[todayBlock(profile), ladderBlock(profile), lastMatchBlock(profile), repeated, syncNote(profile)].filter(Boolean));
  loadTagTrend(repeated);
  return side;
}

function updateLobbyClock(snapshot) {
  const clock = document.getElementById("lobby-clock");
  const started = snapshot?.lobby?.queue_started_ms;
  if (clock && started) clock.textContent = formatClock(Date.now() - started);
}

function renderLobby(view, snapshot, profile) {
  const lobby = snapshot.lobby;
  const { queue_started_ms: _clock, ...stable } = lobby ?? {};
  const signature = JSON.stringify([stable, snapshot.mode, profile, inviteCode(lobby)]);
  if (signature === lobbyView.signature) return;
  lobbyView.signature = signature;

  const phase = lobby?.phase ?? "ready";
  const hero = element("section", `lobby-hero halftone is-${phase}`);
  const own = lobby?.party?.find((member) => member.is_own);
  const banner = picture(own?.card_wide, "halftone-image");
  if (banner) hero.prepend(banner);
  else setHalftone(hero, profile?.history.at(-1)?.map);
  const status = displayText("h2", lobby?.idle && phase === "ready" ? "AWAY" : LOBBY_STATUS[phase], "lobby-status");
  hero.append(labelLine([profile?.name ?? "PLAYER", snapshot.mode]), status);
  if (phase === "queueing") {
    const clock = element("p", "lobby-clock", "0:00");
    clock.id = "lobby-clock";
    clock.setAttribute("role", "timer");
    hero.append(clock);
  }

  const party = element("section", "lobby-party");
  const body = element("div", "lobby-body");
  const side = lobbySide(profile, snapshot);
  const invite = () => side.querySelector(".invite .primary")?.click();
  party.append(partyBanners(lobby, profile, invite));
  body.append(party, side);
  view.replaceChildren(hero, body);
  updateLobbyClock(snapshot);

  // 状態が変わったときだけ動かす。人数や成績の更新では動かさない。
  if (lobbyView.phase !== null && lobbyView.phase !== phase) wipeIn(status);
  lobbyView.phase = phase;
}
