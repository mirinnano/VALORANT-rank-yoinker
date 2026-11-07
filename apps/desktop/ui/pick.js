const ROLES = [
  ["Duelist", "デュエリスト"], ["Initiator", "イニシエーター"],
  ["Controller", "コントローラー"], ["Sentinel", "センチネル"],
];
const FIT_ROWS = 6;
const CLOCK_LATE_MS = 15_000;
const CLOCK_TICK_MS = 250;

const pick = { seen: new Map(), deadline: null, clockValue: null, timer: null };

function roleOf(agent) {
  return images.agents[agent]?.role ?? null;
}

function tile(player) {
  const classes = ["pick-tile"];
  if (player.relation === "own") classes.push("is-own");
  if (!player.agent_locked) classes.push("is-picking");
  const card = element("article", classes.join(" "));

  const portrait = picture(images.agents[player.agent]?.portrait, "portrait");
  if (portrait) card.append(portrait);
  if (player.party) card.append(element("span", "pick-party", partyLetter(player.party)));

  const body = element("div", "pick-tile-body");
  const state = player.agent_locked ? "LOCKED" : player.agent ? "SELECTING" : "WAITING";
  const stateClass = player.agent_locked ? "pick-tile-state is-locked" : "pick-tile-state";
  const rank = element("div", "pick-rank");
  rank.append(rankIcon(player.rank.tier, 22), player.loading ? "読み込み中" : player.rank.name);
  const line = [
    player.win_rate === null ? null : `${player.win_rate}%`,
    player.kd === null ? null : `KD ${player.kd.toFixed(1)}`,
  ].filter(Boolean).join(" / ");

  body.append(
    element("p", stateClass, state),
    element("h3", "pick-agent display", player.agent ?? "－"),
    element("div", player.name_hidden ? "pick-name is-hidden" : "pick-name", player.display_name),
    rank,
    element("div", "pick-line", line),
  );
  if (player.recent_rr.length) body.append(formPips(player.recent_rr));
  card.append(body);

  // エージェントが変わった人だけ知らせる。
  const previous = pick.seen.get(player.key);
  const signature = `${player.agent}:${player.agent_locked}`;
  if (previous !== undefined && previous !== signature) riseIn(body);
  pick.seen.set(player.key, signature);
  return card;
}

function compositionBlock(players) {
  const section = element("section");
  section.append(element("h2", "", "COMP / 構成"));
  const missing = [];
  for (const [role, label] of ROLES) {
    const members = players.filter((player) => player.agent && roleOf(player.agent) === role);
    if (members.length === 0) missing.push(role);
    const row = element("div", members.length ? "comp-role" : "comp-role is-empty-role");
    const icons = element("div", "comp-agents");
    icons.append(...members.map((player) => agentIcon(player.agent, 30)));
    row.append(element("span", "", label), members.length ? icons : element("span", "", "不在"));
    section.append(row);
  }
  const picked = players.filter((player) => player.agent).length;
  if (Object.keys(images.agents).length === 0) {
    section.append(element("p", "fit-note", "役割の情報を読み込めていません。"));
  } else if (picked === players.length && missing.length) {
    const names = missing.map((role) => ROLES.find(([key]) => key === role)[1]).join("、");
    section.append(element("p", "comp-note", `${names}が不在のまま確定しています。`));
  }
  return { section, missing };
}

function spreadBlock(snapshot) {
  const ranked = snapshot.players.filter((player) => player.rank.tier > 0);
  const section = element("section");
  section.append(element("h2", "", "RANK / 味方のランク帯"));
  if (ranked.length === 0) {
    section.append(element("p", "fit-note", "ランクの情報を待っています。"));
    return section;
  }
  const sorted = [...ranked].sort((a, b) => a.rank.tier - b.rank.tier);
  const low = sorted[0].rank;
  const high = sorted[sorted.length - 1].rank;
  const spread = element("div", "rank-spread");
  const pill = (rank, label) => {
    const item = element("div");
    item.append(rankIcon(rank.tier, 36), element("small", "", label), rank.name);
    return item;
  };
  spread.append(pill(low, "下"), pill(snapshot.ally_average ?? low, "平均"), pill(high, "上"));
  section.append(spread);
  return section;
}

/**
 * おすすめの重み。役割別の勝率差は 1 から 2 ポイントと小さいという調査に合わせ、
 * 役割の差は控えめにして、本人が使えるかどうかを重視する。いずれも初期値で、実データには合わせていない。
 */
const WEIGHT = {
  missingRole: { Controller: 0.3, Initiator: 0.22, Sentinel: 0.22, Duelist: 0.16 },
  softMissingRole: 0.1,
  duplicateRole: -0.15,
  mapWin: 0.6,
  ownMap: 1.2,
  ownOverall: 0.8,
  synergy: 0.5,
  selectedByAlly: -0.25,
};
/** 少ない試合で振れないよう、勝率へ足す仮の試合数。 */
const PRIOR = { observed: 20, own: 4, pair: 20 };
const MIN_PAIR_GAMES = 5;
const POOL_SLOTS = 2;
const RECOMMEND_COUNT = 3;
const CONFIRMED_WEIGHT = 1;
const SELECTING_WEIGHT = 0.5;

/** 勝率を 0.5 からの差で返す。試合が少ないほど 0 に近づく。 */
function smoothedEdge(wins, games, prior) {
  return (wins + prior / 2) / (games + prior) - 0.5;
}

function roleCounts(allies) {
  const counts = {};
  for (const ally of allies) {
    const role = roleOf(ally.agent);
    if (!role) continue;
    counts[role] ??= { locked: 0, selecting: 0 };
    counts[role][ally.agent_locked ? "locked" : "selecting"] += 1;
  }
  return counts;
}

function scoreRole(role, counts, reasons) {
  const count = counts[role] ?? { locked: 0, selecting: 0 };
  const label = ROLES.find(([key]) => key === role)?.[1] ?? role;
  if (count.locked + count.selecting === 0) {
    reasons.push(`${label}が不在`);
    return WEIGHT.missingRole[role] ?? 0;
  }
  if (count.locked === 0) {
    reasons.push(`${label}は未確定`);
    return WEIGHT.softMissingRole;
  }
  if (count.locked >= 2) return WEIGHT.duplicateRole;
  return 0;
}

function scoreObserved(stat, reasons) {
  if (!stat?.picks) return 0;
  const edge = smoothedEdge(Math.round((stat.win_percent ?? 0) * stat.picks / 100), stat.picks, PRIOR.observed);
  if (Math.abs(edge) >= 0.03) reasons.push(`マップ勝率 ${stat.win_percent}% (${stat.picks})`);
  return edge * WEIGHT.mapWin;
}

function scoreOwn(agent, stat, summary, reasons) {
  const overall = summary?.agents.find((entry) => entry.name === agent);
  let score = 0;
  if (stat?.own_games) {
    score += smoothedEdge(stat.own_wins, stat.own_games, PRIOR.own) * WEIGHT.ownMap;
    reasons.push(`自分 ${stat.own_wins}勝${stat.own_games - stat.own_wins}敗`);
  }
  if (overall) score += smoothedEdge(overall.wins, overall.games, PRIOR.own) * WEIGHT.ownOverall;
  return score;
}

function scoreSynergy(agent, board, allies, reasons) {
  let score = 0;
  let best = null;
  for (const ally of allies) {
    const pair = board?.pairs.find((entry) => entry.agent === agent && entry.ally === ally.agent);
    if (!pair || pair.games < MIN_PAIR_GAMES) continue;
    const weight = ally.agent_locked ? CONFIRMED_WEIGHT : SELECTING_WEIGHT;
    const edge = smoothedEdge(pair.wins, pair.games, PRIOR.pair);
    score += (edge * WEIGHT.synergy * weight) / allies.length;
    if (edge > 0.03 && (!best || edge > best.edge)) best = { edge, pair };
  }
  if (best) reasons.push(`${best.pair.ally}と ${best.pair.wins}勝${best.pair.games - best.pair.wins}敗`);
  return score;
}

/** マップ、確定と選択中の味方、自分の実績を合わせて候補を順位付けする。 */
function recommend(snapshot) {
  const board = snapshot.pick_board;
  const summary = snapshot.profile?.summary ?? app.profile?.summary;
  const allies = snapshot.players.filter((player) => player.relation !== "own" && player.agent);
  const locked = new Set(allies.filter((ally) => ally.agent_locked).map((ally) => ally.agent));
  const selecting = new Set(allies.filter((ally) => !ally.agent_locked).map((ally) => ally.agent));
  const counts = roleCounts(allies);

  const pool = new Set((summary?.agents ?? []).map((entry) => entry.name));
  const scored = Object.keys(images.agents)
    .filter((agent) => !locked.has(agent))
    .map((agent) => {
      const reasons = [];
      const stat = board?.agents.find((entry) => entry.agent === agent);
      let score = scoreRole(roleOf(agent), counts, reasons);
      score += scoreObserved(stat, reasons);
      score += scoreOwn(agent, stat, summary, reasons);
      score += scoreSynergy(agent, board, allies, reasons);
      if (selecting.has(agent)) {
        score += WEIGHT.selectedByAlly;
        reasons.push("味方が選択中");
      }
      return { agent, score, reasons, inPool: pool.has(agent) };
    })
    .sort((a, b) => b.score - a.score || a.agent.localeCompare(b.agent));

  // 使えるエージェントを先に並べ、最後の枠だけプール外から選ぶ。
  const owned = scored.filter((entry) => entry.inPool).slice(0, POOL_SLOTS);
  const others = scored.filter((entry) => !owned.includes(entry));
  return [...owned, ...others].slice(0, RECOMMEND_COUNT);
}

function recommendBlock(snapshot) {
  const section = element("section", "recommend");
  section.append(element("h2", "", "RECOMMEND / おすすめ"));
  const ranked = recommend(snapshot);
  if (ranked.length === 0) {
    section.append(element("p", "fit-note", "エージェントの情報を読み込むと表示します。"));
    return section;
  }
  ranked.forEach((entry, index) => {
    const row = element("div", index === 0 ? "recommend-row is-top" : "recommend-row");
    const text = element("div");
    const reasons = [entry.inPool ? null : "挑戦", ...entry.reasons].filter(Boolean);
    text.append(element("strong", "", entry.agent), element("small", "", reasons.join(" / ") || "構成への影響は小さい候補です"));
    row.append(agentIcon(entry.agent, index === 0 ? 48 : 36), text);
    section.append(row);
  });
  return section;
}

function fitRow(entry) {
  const row = element("div", "fit-row");
  const own = entry.own_games ? `${entry.own_wins}勝${entry.own_games - entry.own_wins}敗` : "－";
  row.append(
    agentIcon(entry.agent, 28),
    element("span", "", entry.agent),
    element("span", "", entry.picks ? `${entry.pick_percent}%` : "－"),
    element("span", "", entry.win_percent === null ? "－" : `${entry.win_percent}%`),
    element("span", "", own),
  );
  return row;
}

function fitBlock(snapshot) {
  const section = element("section");
  section.append(element("h2", "", "MAP FIT / マップ相性"));
  const board = snapshot.pick_board;
  if (!board || board.agents.length === 0) {
    section.append(element("p", "fit-note", "まだ観測がありません。試合を重ねると、このマップのピック率と勝率が集まります。"));
    return section;
  }
  const head = element("div", "fit-row is-head");
  head.append(element("span"), element("span", "", "エージェント"), element("span", "", "ピック"), element("span", "", "勝率"), element("span", "", "自分"));
  section.append(head, ...board.agents.slice(0, FIT_ROWS).map(fitRow));
  const scope = board.all_ranks ? "全ランク" : "自分のランク帯";
  section.append(element("p", "fit-note", `${scope}のロビーで観測した ${board.matches} 試合の値です。世界全体の統計ではありません。`));
  return section;
}

function formatClock(ms) {
  return String(Math.max(Math.ceil(ms / 1000), 0));
}

function runClock(snapshot, clock) {
  clearInterval(pick.timer);
  const value = snapshot.phase_remaining_ms;
  if (!value) return;
  if (pick.clockValue !== value) {
    pick.clockValue = value;
    pick.deadline = Date.now() + value;
  }
  const update = () => {
    const left = pick.deadline - Date.now();
    clock.textContent = formatClock(left);
    clock.classList.toggle("is-late", left <= CLOCK_LATE_MS);
  };
  update();
  pick.timer = setInterval(update, CLOCK_TICK_MS);
}

/** 骨組みを保ち、更新のたびにランク軸のマーカーが出直さないようにする。 */
function ensurePickLayout(view) {
  if (view.querySelector(".pick-layout")) return;
  match.marks.clear();
  match.heroMap = undefined;
  match.revealTitle = true;
  const axis = createAxis();
  match.lanes.score.classList.add("pick-clock");
  view.replaceChildren(axis, element("div", "pick-layout"));
}

function renderPick(view, snapshot) {
  ensurePickLayout(view);
  updateHero(snapshot);
  updateRankAxis(snapshot);

  const team = element("div", "pick-team");
  team.append(...snapshot.players.map(tile));

  const { section } = compositionBlock(snapshot.players);
  const side = element("div", "pick-side");
  side.append(recommendBlock(snapshot), section, spreadBlock(snapshot), fitBlock(snapshot));

  view.querySelector(".pick-layout").replaceChildren(team, side);
  runClock(snapshot, match.lanes.score);
}

/** 次のエージェント選択に備えて状態を戻す。 */
function resetPick() {
  clearInterval(pick.timer);
  pick.seen.clear();
  pick.clockValue = null;
}
