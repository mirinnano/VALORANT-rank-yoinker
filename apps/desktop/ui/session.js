// 今日のセッション。全モードの最近の試合から、朝5時以降の分を集める。
/** 日付の区切り。深夜の続けたプレイを前日のセッションに含める。 */
const SESSION_DAY_START_HOUR = 5;
/** 次のランクまでの試合数の目安に使う、直近の勝ち試合の数。 */
const SESSION_WIN_SAMPLE = 10;
const SESSION_RR_PER_RANK = 100;
/** Immortal 以上は RR の上限がなく、次のランクまでの距離を出せない。 */
const SESSION_LAST_LADDER_TIER = 24;

function sessionStart(now) {
  const start = new Date(now);
  if (start.getHours() < SESSION_DAY_START_HOUR) start.setDate(start.getDate() - 1);
  start.setHours(SESSION_DAY_START_HOUR, 0, 0, 0);
  return start.getTime();
}

/** `matches` は新しい順。結果は古い順に並べ直して返す。 */
function todaySession(matches, now = Date.now()) {
  const since = sessionStart(now);
  const games = (matches ?? []).filter((entry) => entry.at_ms >= since).reverse();
  const ranked = games.filter((entry) => entry.rr_earned !== null);
  const scored = games.filter((entry) => entry.acs !== null);
  return {
    games: games.length,
    wins: games.filter((entry) => entry.won === true).length,
    losses: games.filter((entry) => entry.won === false).length,
    rr: ranked.reduce((sum, entry) => sum + entry.rr_earned, 0),
    ranked: ranked.length,
    acs: scored.length ? Math.round(scored.reduce((sum, entry) => sum + entry.acs, 0) / scored.length) : null,
    results: games.map((entry) => entry.won),
  };
}

/** 直近の勝ちで得た RR の平均から、次のランクまでに要る勝ち数を見積もる。 */
function winsToRankUp(profile) {
  const { tier, rr } = profile.rank;
  if (tier === 0 || tier >= SESSION_LAST_LADDER_TIER) return null;
  const gains = profile.history.map((point) => point.earned).filter((earned) => earned > 0).slice(-SESSION_WIN_SAMPLE);
  if (gains.length === 0) return null;
  const average = gains.reduce((sum, earned) => sum + earned, 0) / gains.length;
  return { wins: Math.max(Math.ceil((SESSION_RR_PER_RANK - rr) / average), 1), average: Math.round(average) };
}

function sessionResults(results) {
  const strip = element("div", "form session-results");
  strip.setAttribute("role", "img");
  strip.setAttribute("aria-label", `今日の結果、古い順。${results.map((won) => (won === true ? "勝ち" : won === false ? "負け" : "引き分け")).join("、")}`);
  for (const won of results) strip.append(element("i", won === true ? "is-win" : won === false ? "is-loss" : ""));
  return strip;
}

/** 次のランクまでの目安の文。ロビーでは次のランクの欄に出すので、そこから使う。 */
function rankUpNote(profile) {
  const estimate = winsToRankUp(profile);
  if (!estimate) return null;
  return element("p", "lobby-note", `次のランクまで あと約${estimate.wins}勝（最近の勝ちは平均 +${estimate.average} RR）`);
}

function sessionBlock(profile, className = "lobby-block", withEstimate = true) {
  const block = element("section", `${className} session`);
  block.append(element("h2", "", "TODAY / 今日のセッション"));
  const day = todaySession(profile.matches);
  if (day.games === 0) {
    block.append(element("p", "lobby-note", "今日はまだ試合をしていません。朝5時で日付を区切ります。"));
  } else {
    const figures = element("div", "hero-figures");
    figures.append(
      figure(`${day.wins}勝 ${day.losses}敗`, `${day.games}試合`),
      figure(day.ranked ? formatSigned(day.rr) : "－", `RR / ランク戦${day.ranked}試合`, signClass(day.rr)),
      figure(day.acs ?? "－", "平均 ACS"),
    );
    block.append(figures, sessionResults(day.results));
  }
  const note = withEstimate ? rankUpNote(profile) : null;
  if (note) block.append(note);
  return block;
}
