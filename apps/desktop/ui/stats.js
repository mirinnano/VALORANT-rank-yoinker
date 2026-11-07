const ROUND_ROWS = [
  ["pistol", "ピストル"], ["eco", "エコ"], ["half_buy", "ハーフ"], ["full_buy", "フルバイ"],
  ["attack", "攻撃"], ["defense", "防衛"],
];
const MAX_GROUP_ROWS = 8;

function rateRow(label, record) {
  const row = element("div", "rate-row");
  const track = element("div", "rate-track");
  const fill = element("div", "rate-fill");
  fill.style.transform = `scaleX(${record.played ? record.won / record.played : 0})`;
  track.append(fill);
  const value = record.played ? `${roundPercent(record)} (${record.played})` : "－";
  row.append(element("span", "", label), track, element("span", "rate-value", value));
  return row;
}

function groupTable(title, groups, firstCell) {
  const table = element("table");
  table.setAttribute("aria-label", title);
  const head = element("tr");
  for (const [label, className] of [[title, ""], ["試合", "num"], ["勝率", "num"], ["ACS", "num"], ["K/D", "num"]]) {
    head.append(element("th", className, label));
  }
  const thead = element("thead");
  thead.append(head);
  const body = element("tbody");
  for (const group of groups.slice(0, MAX_GROUP_ROWS)) {
    const row = element("tr");
    row.append(
      firstCell(group),
      valueCell(group.games),
      valueCell(percentOf(group.wins, group.games)),
      valueCell(group.acs),
      valueCell(group.kd === null ? null : group.kd.toFixed(2)),
    );
    body.append(row);
  }
  table.append(thead, body);
  return table;
}

function agentCell(group) {
  const cell = element("td");
  const inner = element("div", "score-player");
  inner.append(agentIcon(group.name, 30), group.name);
  cell.append(inner);
  return cell;
}

/** 役割ごとに、先に見る指標。主な役割の順に並べ替える。 */
const ROLE_FOCUS = {
  Duelist: { label: "デュエリスト", keys: ["first", "adr", "acs"] },
  Initiator: { label: "イニシエーター", keys: ["assists", "kast", "adr"] },
  Controller: { label: "コントローラー", keys: ["kast", "survival", "trade"] },
  Sentinel: { label: "センチネル", keys: ["kast", "firstDeath", "clutch"] },
};
const TIME_BANDS = [["朝", 5, 11], ["昼", 11, 17], ["夜", 17, 23]];

function statValues(summary) {
  const percent = (value) => (value === null ? "－" : `${value}%`);
  return {
    acs: [summary.acs ?? "－", "ACS"],
    adr: [summary.adr ?? "－", "ADR"],
    kast: [percent(summary.kast), "KAST"],
    delta: [summary.damage_delta === null ? "－" : formatSigned(summary.damage_delta), "DDΔ / 与被ダメージ差"],
    econ: [summary.econ ?? "－", "ECON / 1000クレジットあたり"],
    kd: [summary.kd === null ? "－" : summary.kd.toFixed(2), "K/D"],
    kda: [summary.kda === null ? "－" : summary.kda.toFixed(2), "KDA"],
    hs: [percent(summary.headshot_percent), "HEADSHOT"],
    first: [formatSigned(summary.first_kill_difference), "先手と被先手の差"],
    firstDeath: [percent(summary.first_death_percent), "被先手率"],
    assists: [summary.assists_per_round === null ? "－" : summary.assists_per_round.toFixed(2), "アシスト / ラウンド"],
    survival: [percent(summary.survival_percent), "生存率"],
    trade: [percent(summary.trade_percent), "トレードされた死"],
    clutch: [summary.clutch.played ? `${summary.clutch.won}/${summary.clutch.played}` : "－", "クラッチ"],
  };
}

function mainRole(summary) {
  const counts = new Map();
  for (const agent of summary.agents) {
    const role = images.agents[agent.name]?.role;
    if (role) counts.set(role, (counts.get(role) ?? 0) + agent.games);
  }
  return [...counts].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
}

function orderedKeys(role) {
  const all = ["acs", "adr", "kast", "delta", "econ", "kd", "kda", "hs", "first", "firstDeath", "assists", "survival", "trade", "clutch"];
  const focus = ROLE_FOCUS[role]?.keys ?? [];
  return [...focus, ...all.filter((key) => !focus.includes(key))];
}

function compareBlock(trend) {
  const box = element("div", "compare");
  const cell = (text, className = "") => element("span", className, text);
  box.append(
    cell("", "head"), cell(`直近${trend.recent_games}試合`, "head"), cell("それ以前", "head"),
    element("div", "", "ACS"), cell(trend.recent_acs ?? "－"), cell(trend.previous_acs ?? "－"),
    element("div", "", "KAST"), cell(`${trend.recent_kast ?? "－"}%`), cell(`${trend.previous_kast ?? "－"}%`),
  );
  return box;
}

function acsBars(lines) {
  const bars = element("div", "acs-bars");
  bars.setAttribute("role", "img");
  bars.setAttribute("aria-label", `直近${lines.length}試合の ACS。白は勝ち、赤は負け`);
  const highest = Math.max(1, ...lines.map((line) => line.acs ?? 0));
  for (const line of [...lines].reverse()) {
    const bar = element("div", line.won === false ? "acs-bar is-loss" : "acs-bar");
    bar.style.height = `${Math.max(((line.acs ?? 0) / highest) * 100, 4)}%`;
    bar.title = `ACS ${line.acs ?? "－"}`;
    bars.append(bar);
  }
  return bars;
}

function timeBandRows(lines) {
  const records = new Map([...TIME_BANDS.map(([name]) => [name, { played: 0, won: 0 }]), ["深夜", { played: 0, won: 0 }]]);
  for (const line of lines) {
    if (line.won === null || !line.started_ms) continue;
    const hour = new Date(line.started_ms).getHours();
    const band = TIME_BANDS.find(([, from, to]) => hour >= from && hour < to)?.[0] ?? "深夜";
    const record = records.get(band);
    record.played += 1;
    record.won += line.won ? 1 : 0;
  }
  return [...records].map(([name, record]) => rateRow(name, record));
}

function renderStats(view, summary) {
  if (!summary) {
    view.replaceChildren(noticeBox("統計を集めています", "コンペティティブの試合を 1 試合ずつ取得しています。接続中はこの画面が順に更新されます。"));
    return;
  }

  const role = mainRole(summary);
  const values = statValues(summary);
  const figures = element("div", "stats-figures");
  figures.append(figure(percentOf(summary.wins, summary.games) ?? "－", `勝率 / ${summary.games}試合`));
  figures.append(...orderedKeys(role).map((key) => figure(...values[key])));

  const rounds = element("section");
  rounds.append(element("h2", "", `ROUNDS / ${summary.rounds}ラウンド`));
  rounds.append(...ROUND_ROWS.map(([key, label]) => rateRow(label, summary[key])));

  const form = element("section");
  form.append(element("h2", "", "FORM / ACS の推移"), acsBars(summary.lines));
  if (summary.trend) form.append(compareBlock(summary.trend));

  const mental = element("section");
  mental.append(
    element("h2", "", "AFTER / 直前の試合の結果と次の勝率"),
    rateRow("勝った次", summary.after_win),
    rateRow("負けた次", summary.after_loss),
  );

  const time = element("section");
  time.append(element("h2", "", "TIME / 時間帯別の勝率"), ...timeBandRows(summary.lines));

  const agents = element("section");
  agents.append(groupTable("エージェント", summary.agents, agentCell));
  const maps = element("section");
  maps.append(groupTable("マップ", summary.maps, (group) => element("td", "name", group.name)));

  const grid = element("div", "stats-grid");
  grid.append(form, rounds, mental, time, agents, maps);
  const roleLabel = ROLE_FOCUS[role]?.label;
  view.replaceChildren(
    displayText("h2", `Last ${summary.games}`, "stats-title"),
    labelLine([roleLabel ? `主な役割 ${roleLabel}` : null, "役割で重視される指標を先に表示"]),
    figures,
    grid,
    element("p", "stats-note", "KAST のトレードは、倒されてから 5 秒以内に味方が相手を倒した場合としています。攻守は爆弾の設置者から判定します。クラッチはセージの蘇生を考慮していません。"),
  );
}
