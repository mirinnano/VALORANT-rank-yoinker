const STATE_LABELS = { menus: "メニュー", pregame: "エージェント選択", in_game: "試合中" };
const RELATION_LABELS = { own: "あなた", party: "同じパーティ", ally: "味方", enemy: "相手チーム" };
const SLOTS = 26;
const STACK_RISE = 18;
const STACK_SHIFT = 8;
const LOCK_FLASH = [{ backgroundColor: "rgb(221 224 41 / 30%)" }, { backgroundColor: "rgb(221 224 41 / 0%)" }];
const LOCK_RISE = [{ transform: "translateY(3px)" }, { transform: "translateY(0)" }];
const MARK_STAGGER_MS = 35;

const COLUMNS = [
  ["組", "party"], ["エージェント", "agent"], ["プレイヤー", ""], ["ランク", ""], ["RR", "num"],
  ["最高", "optional"], ["勝率", "num"], ["K/D", "num"], ["ACS", "num"], ["ADR", "num"],
  ["KAST", "num"], ["HS%", "num"], ["直近5戦", ""],
  ["Lv", "num optional"], ["遭遇", "num optional"],
];

const match = { live: { match: null, ally: 0, enemy: 0, rounds: [] }, revealTitle: false, loading: new Set(), selected: null, marks: new Map(), lanes: null, agents: new Map(), signature: "" };

const isEnemy = (player) => player.relation === "enemy";

function slotOf(tier) {
  return tier < FIRST_TIER ? 0 : tier - FIRST_TIER + 1;
}

function positionOf(tier) {
  return ((slotOf(tier) + 0.5) / SLOTS) * 100;
}

function createBands() {
  const bands = element("div", "bands");
  bands.setAttribute("aria-hidden", "true");
  const unranked = element("div", "band", "－");
  unranked.style.setProperty("--band", UNRANKED_COLOR);
  unranked.style.flex = "1";
  bands.append(unranked);
  RANK_BANDS.forEach(([name, color], index) => {
    const band = element("div", "band", index === RANK_BANDS.length - 1 ? "RAD" : name.toUpperCase());
    band.style.setProperty("--band", color);
    band.style.flex = String(index === RANK_BANDS.length - 1 ? 1 : TIERS_PER_BAND);
    bands.append(band);
  });
  return bands;
}

function createAxis() {
  const axis = element("section", "axis halftone");
  const head = element("div", "axis-head");
  const heading = element("div");
  const title = element("h2", "axis-map display");
  const record = element("p", "axis-record label");
  heading.append(title, record);
  const score = element("div", "axis-score");
  const flow = element("div", "live-flow");
  const scoreBlock = element("div");
  scoreBlock.append(score, flow);
  head.append(heading, scoreBlock);

  const grid = element("div", "axis-grid");
  grid.setAttribute("role", "img");
  const ally = element("div", "lane ally");
  const enemy = element("div", "lane enemy");
  const allyLabel = element("span", "axis-label", "ALLY");
  const enemyLabel = element("span", "axis-label", "ENEMY");
  grid.append(allyLabel, ally, element("span"), createBands(), enemyLabel, enemy);
  axis.append(head, grid);
  match.lanes = { axis: grid, root: axis, title, record, score, flow, ally, enemy, allyLabel, enemyLabel };
  return axis;
}

const LOSS_BONUS = [1900, 2400, 2900];
const HALF_ROUNDS = 12;

/** スコアの増え方から、ラウンドの勝ち負けを順に記録する。 */
function trackRounds(snapshot) {
  const live = match.live;
  if (live.match !== snapshot.match_id) {
    Object.assign(live, { match: snapshot.match_id, ally: snapshot.score[0], enemy: snapshot.score[1], rounds: [] });
    return;
  }
  const allyWins = snapshot.score[0] - live.ally;
  const enemyWins = snapshot.score[1] - live.enemy;
  if (allyWins === 1 && enemyWins === 0) live.rounds.push(true);
  else if (enemyWins === 1 && allyWins === 0) live.rounds.push(false);
  else if (allyWins !== 0 || enemyWins !== 0) live.rounds.length = 0;
  live.ally = snapshot.score[0];
  live.enemy = snapshot.score[1];
}

function trailingCount(rounds, value) {
  let count = 0;
  for (let index = rounds.length - 1; index >= 0 && rounds[index] === value; index -= 1) count += 1;
  return count;
}

function lossBonus(losses) {
  return LOSS_BONUS[Math.min(losses, LOSS_BONUS.length) - 1];
}

/** スコアだけから分かる流れを出す。経済は連敗数からの推定で、実際の所持金ではない。 */
function updateLiveFlow(snapshot) {
  trackRounds(snapshot);
  const { flow } = match.lanes;
  const { rounds } = match.live;
  const played = snapshot.score[0] + snapshot.score[1];
  const next = played + 1;
  const parts = [];

  if (played === 0 || played === HALF_ROUNDS) parts.push(element("span", "", `次 R${next} ピストル`));
  else parts.push(element("span", "", `次 R${next}`));

  if (rounds.length) {
    const strip = element("span", "live-rounds");
    strip.setAttribute("aria-label", `観測した直近のラウンド、${rounds.filter(Boolean).length}勝`);
    strip.append(...rounds.slice(-10).map((won) => element("i", won ? "" : "is-loss")));
    parts.push(strip);

    const wins = trailingCount(rounds, true);
    const losses = trailingCount(rounds, false);
    if (wins >= 2) parts.push(element("span", "", `${wins}連勝`));
    if (losses >= 2) parts.push(element("span", "", `${losses}連敗`));
    // 前半と後半の間は経済がリセットされるので推定しない。
    if (played !== HALF_ROUNDS) {
      if (losses) parts.push(element("span", "", `味方の推定ボーナス ${lossBonus(losses)}`));
      if (wins) parts.push(element("span", "", `相手の推定ボーナス ${lossBonus(wins)}`));
    }
  }
  flow.replaceChildren(...parts);
}

function updateHero(snapshot) {
  const { root, title, record, score } = match.lanes;
  record.textContent = mapRecord(snapshot.profile ?? app.profile, snapshot.map) ?? "";
  if (match.heroMap !== snapshot.map) {
    match.heroMap = snapshot.map;
    setHalftone(root, snapshot.map);
    title.replaceChildren(snapshot.map ?? "MATCH", element("span", "period"));
    if (match.revealTitle) {
      match.revealTitle = false;
      wipeIn(title);
    }
  }
  if (snapshot.state === "pregame") return;
  if (!snapshot.score) {
    score.replaceChildren();
    return;
  }
  const value = `${snapshot.score[0]}:${snapshot.score[1]}`;
  if (score.dataset.value === value) return;
  score.dataset.value = value;
  score.replaceChildren(String(snapshot.score[0]), element("span", "", " : "), String(snapshot.score[1]));
  updateLiveFlow(snapshot);
}

function highlight(key, active) {
  match.marks.get(key)?.classList.toggle("is-hot", active);
  document.querySelector(`.player[data-key="${key}"]`)?.classList.toggle("is-hot", active);
}

function placeMark(mark, tier, stack, side) {
  const y = (side === "enemy" ? 1 : -1) * stack * STACK_RISE;
  const x = stack * STACK_SHIFT;
  mark.style.transform = `translate(calc(${positionOf(tier)}cqw + ${x}px), ${y}px)`;
}

/** 新しいマーカーを軸から広げる。相手チームが見えた瞬間を示す。 */
function revealMark(mark, side, order) {
  const reach = side === "enemy" ? "-10px" : "10px";
  const from = motionReduced() ? { opacity: 0 } : { opacity: 0, translate: `0 ${reach}` };
  const to = motionReduced() ? { opacity: 1 } : { opacity: 1, translate: "0 0" };
  mark.animate([from, to], {
    duration: 240,
    delay: motionReduced() ? 0 : order * MARK_STAGGER_MS,
    easing: EASE_OUT,
    fill: "backwards",
  });
}

function updateMarks(players) {
  const stacks = new Map();
  const seen = new Set();
  const added = { ally: 0, enemy: 0 };
  for (const player of players) {
    const side = isEnemy(player) ? "enemy" : "ally";
    const stackKey = `${side}-${player.rank.tier}`;
    const stack = stacks.get(stackKey) ?? 0;
    stacks.set(stackKey, stack + 1);

    let mark = match.marks.get(player.key);
    if (!mark) {
      mark = element("button", "mark");
      mark.type = "button";
      mark.tabIndex = -1;
      mark.setAttribute("aria-hidden", "true");
      mark.addEventListener("click", () => selectPlayer(player.key));
      mark.addEventListener("mouseenter", () => highlight(player.key, true));
      mark.addEventListener("mouseleave", () => highlight(player.key, false));
      match.marks.set(player.key, mark);
      revealMark(mark, side, added[side]);
      added[side] += 1;
    }
    if (mark.parentElement !== match.lanes[side]) match.lanes[side].append(mark);
    mark.classList.toggle("is-own", player.relation === "own");
    mark.title = `${player.display_name}、${player.rank.name}`;
    if (mark.dataset.agent !== player.agent) {
      mark.dataset.agent = player.agent ?? "";
      mark.replaceChildren(agentIcon(player.agent, 24));
    }
    placeMark(mark, player.rank.tier, stack, side);
    seen.add(player.key);
  }
  for (const [key, mark] of match.marks) {
    if (!seen.has(key)) {
      mark.remove();
      match.marks.delete(key);
    }
  }
}

function updateOverlays(lane, players, average) {
  lane.querySelectorAll(".party-link, .average").forEach((node) => node.remove());

  if (average) {
    const tick = element("div", "average");
    tick.title = `平均 ${average.name}`;
    tick.style.transform = `translateX(${positionOf(average.tier)}cqw)`;
    lane.prepend(tick);
  }

  const parties = new Map();
  for (const player of players.filter((entry) => entry.party)) {
    parties.set(player.party, [...(parties.get(player.party) ?? []), positionOf(player.rank.tier)]);
  }
  for (const [party, positions] of parties) {
    const from = Math.min(...positions);
    const link = element("div", "party-link");
    link.style.left = `${from}%`;
    link.style.width = `${Math.max(Math.max(...positions) - from, 0.6)}%`;
    link.append(element("b", "", partyLetter(party)));
    lane.append(link);
  }
}

function valueCell(value, className = "num") {
  const absent = value === null || value === undefined;
  return element("td", absent ? `${className} absent` : className, absent ? "－" : String(value));
}

function rankCell(rank) {
  const cell = element("td");
  const inner = element("div", "rank-cell");
  inner.append(rankIcon(rank.tier, 30), element("span", "", rank.name));
  cell.append(inner);
  return cell;
}

function loadingCell() {
  return element("td", "absent", "読み込み中");
}

function playerRow(player) {
  const row = element("tr", "player");
  row.dataset.key = player.key;
  row.dataset.relation = player.relation;
  row.tabIndex = -1;
  row.setAttribute("aria-selected", String(player.key === match.selected));

  const name = element("td", player.name_hidden ? "name is-hidden" : "name", player.display_name);
  const agent = element("td", player.agent_locked ? "agent" : "agent is-picking");
  const agentName = element("span", "agent-cell");
  agentName.append(
    agentIcon(player.agent, 30),
    player.agent ? (player.agent_locked ? player.agent : `${player.agent}（選択中）`) : "選択中",
  );
  agent.append(agentName);
  const form = element("td");
  if (player.recent_rr.length) form.append(formPips(player.recent_rr));

  const rank = player.loading ? loadingCell() : rankCell(player.rank);
  // 読み込み中からランクへ変わった行だけ知らせる。
  if (player.loading) match.loading.add(player.key);
  else if (match.loading.delete(player.key)) riseIn(rank.firstElementChild);

  row.append(
    element("td", "party", partyLetter(player.party)),
    agent,
    name,
    rank,
    valueCell(player.rank.tier === 0 ? null : player.rank.rr),
    element("td", "optional", player.peak.name),
    valueCell(player.win_rate === null ? null : `${player.win_rate}% (${player.games})`),
    valueCell(player.kd === null ? null : player.kd.toFixed(2)),
    valueCell(player.acs),
    valueCell(player.adr),
    valueCell(player.kast === null ? null : `${player.kast}%`),
    valueCell(player.headshot_percent),
    form,
    valueCell(player.level, "num optional"),
    valueCell(player.encounters > 0 ? `${player.encounters}回` : null, "num optional"),
  );

  row.addEventListener("click", () => selectPlayer(player.key));
  row.addEventListener("mouseenter", () => highlight(player.key, true));
  row.addEventListener("mouseleave", () => highlight(player.key, false));

  const previous = match.agents.get(player.key);
  const signature = `${player.agent}:${player.agent_locked}`;
  if (previous && previous !== signature) {
    agent.animate(LOCK_FLASH, { duration: 400, easing: "ease-out" });
    if (!motionReduced()) agentName.animate(LOCK_RISE, { duration: 180, easing: EASE_OUT });
  }
  match.agents.set(player.key, signature);
  return row;
}

function teamGroup(label, players, average) {
  const group = element("tbody");
  const head = element("tr");
  const cell = element("th");
  cell.colSpan = COLUMNS.length;
  cell.scope = "rowgroup";
  cell.append(`${label} / ${players.length}`);
  if (average) {
    cell.append(" / AVG ");
    cell.append(element("b", "", average.name));
  }
  head.append(cell);
  group.append(head, ...players.map(playerRow));
  return group;
}

function fact(list, term, value) {
  list.append(element("dt", "", term), element("dd", value === null || value === undefined ? "absent" : "", value ?? "－"));
}

function relativeTime(ms) {
  if (ms === null) return null;
  const days = Math.floor((Date.now() - ms) / 86_400_000);
  if (days < 1) return "今日";
  return days === 1 ? "昨日" : `${days}日前`;
}

function renderDetail(pane, player) {
  if (!player) {
    pane.replaceChildren(element("p", "", "プレイヤーを選ぶと詳細を表示します"));
    return;
  }
  const rankFacts = element("dl");
  fact(rankFacts, "今シーズン", player.rank.tier ? `${player.rank.name}、${player.rank.rr} RR` : player.rank.name);
  fact(rankFacts, "前シーズン", player.previous_rank.name);
  fact(rankFacts, "最高", player.peak_act ? `${player.peak.name} (${player.peak_act})` : player.peak.name);
  fact(rankFacts, "勝率", player.win_rate === null ? null : `${player.win_rate}%、${player.games}試合`);

  const recent = element("dl");
  fact(recent, "K/D", player.kd === null ? null : player.kd.toFixed(2));
  fact(recent, "ヘッドショット", player.headshot_percent === null ? null : `${player.headshot_percent}%`);
  fact(recent, "ACS", player.acs);
  fact(recent, "ADR", player.adr);
  fact(recent, "KAST", player.kast === null ? null : `${player.kast}%`);
  fact(recent, "DDΔ", player.damage_delta === null ? null : formatSigned(player.damage_delta));
  fact(recent, "Econ", player.econ);
  fact(recent, "先手率", player.first_kill_percent === null ? null : `${player.first_kill_percent}%`);
  fact(recent, "被先手率", player.first_death_percent === null ? null : `${player.first_death_percent}%`);
  fact(recent, "獲得 RR", player.earned_rr === null ? null : formatSigned(player.earned_rr));
  fact(recent, "最後のプレイ", relativeTime(player.last_active_ms));
  if (player.recent_rr.length) {
    recent.append(element("dt", "", `直近${player.recent_rr.length}試合`));
    const form = element("dd");
    form.append(formPips(player.recent_rr));
    recent.append(form);
  }

  const subtitle = [RELATION_LABELS[player.relation], player.agent].filter(Boolean).join("、");
  const hero = element("div", "detail-hero halftone");
  hero.append(rankIcon(player.rank.tier, 56, "large"));
  const portrait = picture(images.agents[player.agent]?.portrait, "portrait");
  if (portrait) hero.prepend(portrait);

  const body = element("div", "detail-body");
  const nodes = [
    element("h2", "", player.display_name),
    element("p", "label", subtitle),
    rankFacts,
    element("h3", "", "直近のコンペティティブ"),
    recent,
    element("h3", "", "あなたとの関係"),
  ];
  if (player.relation === "own") nodes.push(...ownSummaryNodes(match.map));
  const relation = element("dl");
  fact(relation, "遭遇", player.encounters > 0 ? `過去 ${player.encounters} 回` : "はじめて");
  fact(relation, "レベル", player.level ?? "非公開");
  nodes.push(relation);
  if (player.name_hidden) {
    nodes.push(element("p", "privacy", "このプレイヤーは Streamer Mode のため、名前をエージェント名で表示しています。"));
  }
  body.append(...nodes);
  pane.replaceChildren(hero, body);
}

/** 自分だけに出す、保存済みの試合の集計。 */
function ownSummaryNodes(map) {
  const summary = app.profile?.summary;
  if (!summary) return [];
  const all = element("dl");
  fact(all, "勝率", percentOf(summary.wins, summary.games));
  fact(all, "ACS / ADR", `${summary.acs ?? "－"} / ${summary.adr ?? "－"}`);
  fact(all, "KAST", summary.kast === null ? null : `${summary.kast}%`);
  fact(all, "K/D", summary.kd === null ? null : summary.kd.toFixed(2));
  fact(all, "DDΔ / Econ", `${summary.damage_delta === null ? "－" : formatSigned(summary.damage_delta)} / ${summary.econ ?? "－"}`);
  fact(all, "先手 / 被先手", `${summary.first_kill_percent ?? "－"}% / ${summary.first_death_percent ?? "－"}%`);
  fact(all, "クラッチ", summary.clutch.played ? `${summary.clutch.won}/${summary.clutch.played}` : null);
  fact(all, "トレードされた死", summary.trade_percent === null ? null : `${summary.trade_percent}%`);
  fact(all, "ピストル", roundPercent(summary.pistol));
  fact(all, "攻撃 / 防衛", `${roundPercent(summary.attack) ?? "－"} / ${roundPercent(summary.defense) ?? "－"}`);
  fact(all, "エコ / フルバイ", `${roundPercent(summary.eco) ?? "－"} / ${roundPercent(summary.full_buy) ?? "－"}`);
  const nodes = [element("h3", "", `直近${summary.games}試合の自分`), all];

  const onMap = summary.maps.find((entry) => entry.name === map);
  if (onMap) {
    const mapFacts = element("dl");
    fact(mapFacts, "勝率", `${onMap.wins}勝${onMap.games - onMap.wins}敗`);
    fact(mapFacts, "ACS / K/D", `${onMap.acs ?? "－"} / ${onMap.kd?.toFixed(2) ?? "－"}`);
    nodes.push(element("h3", "", `${map} の自分`), mapFacts);
  }
  return nodes;
}

function selectPlayer(key, focus = false) {
  match.selected = key;
  for (const row of document.querySelectorAll(".player")) {
    const selected = Number(row.dataset.key) === key;
    row.setAttribute("aria-selected", String(selected));
    row.tabIndex = selected ? 0 : -1;
    if (selected && focus) row.focus();
  }
  for (const [markKey, mark] of match.marks) mark.classList.toggle("is-selected", markKey === key);
  const player = match.players?.find((entry) => entry.key === key);
  renderDetail(document.querySelector(".detail"), player);
}

function moveSelection(offset) {
  const keys = match.players.map((player) => player.key);
  const next = keys[keys.indexOf(match.selected) + offset];
  if (next !== undefined) selectPlayer(next, true);
}

function ensureMatchLayout(view) {
  if (view.querySelector(".table-pane")) return;
  match.marks.clear();
  match.heroMap = undefined;
  match.revealTitle = true;
  match.loading.clear();
  const pane = element("div", "table-pane");
  const detail = element("aside", "detail");
  detail.setAttribute("aria-label", "選んだプレイヤーの詳細");
  view.replaceChildren(createAxis(), pane, detail);
  pane.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") moveSelection(1);
    else if (event.key === "ArrowUp") moveSelection(-1);
    else return;
    event.preventDefault();
  });
}

function renderTable(view, snapshot) {
  const allies = snapshot.players.filter((player) => !isEnemy(player));
  const enemies = snapshot.players.filter(isEnemy);
  const focused = document.activeElement?.classList.contains("player");

  const table = element("table");
  table.setAttribute("role", "grid");
  table.setAttribute("aria-label", "試合のプレイヤー");
  const head = element("tr");
  for (const [label, className] of COLUMNS) head.append(element("th", className, label));
  const thead = element("thead");
  thead.append(head);
  table.append(thead, teamGroup("ALLY", allies, snapshot.ally_average));
  if (enemies.length) table.append(teamGroup("ENEMY", enemies, snapshot.enemy_average));
  view.querySelector(".table-pane").replaceChildren(table);

  const known = snapshot.players.some((player) => player.key === match.selected);
  const own = snapshot.players.find((player) => player.relation === "own");
  selectPlayer(known ? match.selected : own?.key ?? snapshot.players[0].key, focused);
}

/** ランク軸を更新する。対戦画面とピック画面で共有する。 */
function updateRankAxis(snapshot) {
  const allies = snapshot.players.filter((player) => !isEnemy(player));
  const enemies = snapshot.players.filter(isEnemy);
  updateMarks(snapshot.players);
  updateOverlays(match.lanes.ally, allies, snapshot.ally_average);
  updateOverlays(match.lanes.enemy, enemies, snapshot.enemy_average);

  const single = enemies.length === 0;
  match.lanes.enemy.classList.toggle("is-single", single);
  match.lanes.enemyLabel.textContent = single ? "" : "ENEMY";

  const summary = [
    snapshot.ally_average && `味方の平均は ${snapshot.ally_average.name}`,
    snapshot.enemy_average && `相手の平均は ${snapshot.enemy_average.name}`,
  ].filter(Boolean).join("、");
  match.lanes.axis.setAttribute("aria-label", summary || "ランクの分布");
}

function renderMatch(view, snapshot) {
  if (snapshot.state !== "pregame") resetPick();
  if (snapshot.state === "pregame" && snapshot.players.length > 0) {
    match.signature = "";
    match.players = snapshot.players;
    view.classList.add("is-empty");
    renderPick(view, snapshot);
    return;
  }

  if (snapshot.players.length === 0) {
    match.signature = "";
    match.agents.clear();
    match.selected = null;
    view.classList.add("is-empty");
    view.replaceChildren(noticeBox("試合はまだ始まっていません", "エージェント選択が始まると、この画面に切り替わります。"));
    return;
  }

  // 同じ内容なら描き直さない。選択やスクロールを保つ。
  if (view.querySelector(".table-pane")) updateHero(snapshot);
  const signature = JSON.stringify(snapshot.players) + snapshot.state;
  if (signature === match.signature) return;
  match.signature = signature;
  match.players = snapshot.players;
  match.map = snapshot.map;

  view.classList.remove("is-empty");
  ensureMatchLayout(view);

  updateRankAxis(snapshot);

  match.summary = snapshot.profile?.summary ?? app.profile?.summary ?? null;

  updateHero(snapshot);
  renderTable(view, snapshot);
}
