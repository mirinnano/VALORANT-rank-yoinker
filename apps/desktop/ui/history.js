const HISTORY_COLUMNS = [
  ["プレイヤー", ""], ["エージェント", ""], ["マップ", ""], ["立場", ""], ["ランク", ""],
  ["遭遇", "num"], ["日時", "num"],
];
const RELATION_NAMES = { ally: "味方", enemy: "相手" };

function historyRow(entry, format) {
  const row = element("tr");
  const cell = (value, className = "") => element("td", value ? className : `${className} absent`, value ?? "－");
  const agent = element("td");
  const agentInner = element("div", "score-player");
  agentInner.append(agentIcon(entry.agent, 24), entry.agent ?? "－");
  agent.append(agentInner);
  row.append(
    element("td", "name", entry.display_name),
    agent,
    cell(entry.map),
    cell(RELATION_NAMES[entry.relation]),
    rankCell({ tier: tierOfName(entry.rank), name: entry.rank }),
    cell(`${entry.times_met}回`, "num"),
    cell(format.format(new Date(entry.seen_at_ms)), "num"),
  );
  return row;
}

function historyTable(entries) {
  const format = new Intl.DateTimeFormat("ja-JP", { dateStyle: "short", timeStyle: "short" });
  const table = element("table");
  table.setAttribute("aria-label", "遭遇したプレイヤー");
  const head = element("tr");
  for (const [label, className] of HISTORY_COLUMNS) head.append(element("th", className, label));
  const thead = element("thead");
  thead.append(head);
  const body = element("tbody");
  body.append(...entries.map((entry) => historyRow(entry, format)));
  table.append(thead, body);
  return table;
}

function matches(entry, query) {
  const fields = [entry.display_name, entry.agent, entry.map, entry.rank];
  return fields.some((field) => field?.toLowerCase().includes(query));
}

function renderHistory(view, entries) {
  if (entries.length === 0) {
    view.replaceChildren(noticeBox("遭遇履歴はまだありません", "試合に参加すると、一緒になったプレイヤーがここに残ります。"));
    return;
  }

  const results = element("div");
  const filter = element("label", "filter", "絞り込み");
  const input = element("input");
  input.type = "search";
  input.placeholder = "名前、エージェント、マップ、ランク";
  filter.append(input);

  const apply = () => {
    const query = input.value.trim().toLowerCase();
    const shown = query ? entries.filter((entry) => matches(entry, query)) : entries;
    results.replaceChildren(
      shown.length
        ? historyTable(shown)
        : noticeBox("一致する履歴はありません", "別の言葉で絞り込むか、入力を消してください。"),
    );
  };
  input.addEventListener("input", apply);
  apply();
  view.replaceChildren(displayText("h2", "Encounters", "history-title"), filter, results);
}
