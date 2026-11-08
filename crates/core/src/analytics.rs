//! 試合詳細から指標を計算する。
//!
//! 計算の定義:
//! - ACS は `stats.score` をラウンド数で割った値。ADR は与えたダメージの合計をラウンド数で割った値。
//! - KAST はキル、アシスト、生存、トレードのいずれかがあったラウンドの割合。
//!   トレードは、倒されてから [`TRADE_WINDOW_MS`] 以内に味方が相手を倒した場合とする。
//! - 経済は自分の装備の価値で分ける。ピストルは第 1 ラウンドと第 13 ラウンド。
//! - 攻守は、爆弾を設置したプレイヤーのチームを攻撃側として判定する。設置がなければ集計しない。
//! - DDΔ は 1 ラウンドあたりの与ダメージと被ダメージの差。Econ は 1,000 クレジットあたりの与ダメージ。
//! - クラッチは、味方が全滅して自分だけが残り、相手が 1 人以上いる場面。セージの蘇生は追跡しない。

use crate::source::GameCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

const TRADE_WINDOW_MS: i64 = 5_000;
const ECO_LOADOUT: i64 = 2_000;
const FULL_LOADOUT: i64 = 3_900;
const REGULATION_HALF: usize = 12;
const REGULATION_ROUNDS: usize = 24;
const TREND_RECENT: usize = 5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundRecord {
    pub played: u32,
    pub won: u32,
}

impl RoundRecord {
    fn add(&mut self, won: bool) {
        self.played += 1;
        self.won += u32::from(won);
    }

    fn merge(&mut self, other: RoundRecord) {
        self.played += other.played;
        self.won += other.won;
    }

    pub fn percent(self) -> Option<u32> {
        (self.played > 0).then(|| self.won * 100 / self.played)
    }
}

/// 1 試合の指標。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchMetrics {
    pub match_id: String,
    pub started_ms: i64,
    pub map: Option<String>,
    pub agent: Option<String>,
    pub won: Option<bool>,
    pub rounds: u32,
    pub score: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: i64,
    pub damage_received: i64,
    pub credits_spent: i64,
    pub headshots: u32,
    pub hits: u32,
    pub kast_rounds: u32,
    pub first_kills: u32,
    pub first_deaths: u32,
    pub multi_kills: u32,
    pub survived_rounds: u32,
    pub traded_deaths: u32,
    pub clutch: RoundRecord,
    pub pistol: RoundRecord,
    pub eco: RoundRecord,
    pub half_buy: RoundRecord,
    pub full_buy: RoundRecord,
    pub attack: RoundRecord,
    pub defense: RoundRecord,
}

fn percent(part: u32, whole: u32) -> Option<u32> {
    (whole > 0).then(|| part * 100 / whole)
}

fn per_round(total: i64, rounds: u32) -> Option<u32> {
    (rounds > 0).then(|| (total / i64::from(rounds)) as u32)
}

impl MatchMetrics {
    pub fn acs(&self) -> Option<u32> {
        per_round(i64::from(self.score), self.rounds)
    }

    pub fn adr(&self) -> Option<u32> {
        per_round(self.damage, self.rounds)
    }

    pub fn kast(&self) -> Option<u32> {
        percent(self.kast_rounds, self.rounds)
    }

    pub fn headshot_percent(&self) -> Option<u32> {
        percent(self.headshots, self.hits)
    }

    /// 1 ラウンドあたりの与ダメージと被ダメージの差。
    pub fn damage_delta(&self) -> Option<i32> {
        (self.rounds > 0).then(|| ((self.damage - self.damage_received) / i64::from(self.rounds)) as i32)
    }

    /// 1,000 クレジットあたりの与ダメージ。
    pub fn econ(&self) -> Option<u32> {
        (self.credits_spent > 0).then(|| (self.damage * 1000 / self.credits_spent) as u32)
    }

    pub fn first_kill_percent(&self) -> Option<u32> {
        percent(self.first_kills, self.rounds)
    }

    pub fn first_death_percent(&self) -> Option<u32> {
        percent(self.first_deaths, self.rounds)
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn int(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn kill_time(kill: &Value) -> i64 {
    kill.get("timeSinceRoundStartMillis")
        .or_else(|| kill.get("roundTime"))
        .and_then(Value::as_i64)
        .unwrap_or(i64::MAX)
}

pub fn round_kills(round: &Value) -> Vec<&Value> {
    let mut kills: Vec<&Value> =
        list(round, "playerStats").iter().flat_map(|player| list(player, "kills")).collect();
    kills.sort_by_key(|kill| kill_time(kill));
    kills
}

/// 前半に攻撃したチームを、爆弾の設置者から求める。
fn first_half_attackers<'a>(details: &'a Value, teams: &HashMap<&str, &'a str>) -> Option<&'a str> {
    list(details, "roundResults").iter().enumerate().find_map(|(index, round)| {
        let team = *teams.get(text(round, "bombPlanter")?)?;
        if index < REGULATION_HALF {
            return Some(team);
        }
        if index >= REGULATION_ROUNDS {
            return None;
        }
        teams.values().copied().find(|other| *other != team)
    })
}

/// 味方が全滅して自分だけが残った場面の、残りの相手の人数を返す。キルの順に生存者を減らして調べる。
pub fn clutch_opponents(puuid: &str, team: Option<&str>, teams: &HashMap<&str, &str>, kills: &[&Value]) -> Option<u32> {
    let team = team?;
    let mut allies = teams.values().filter(|other| **other == team).count();
    let mut enemies = teams.len() - allies;
    if allies < 2 {
        return None;
    }
    for kill in kills {
        let Some(victim) = text(kill, "victim") else { continue };
        if victim == puuid {
            return None;
        }
        if teams.get(victim).copied() == Some(team) {
            allies = allies.saturating_sub(1);
        } else {
            enemies = enemies.saturating_sub(1);
        }
        if allies == 1 {
            return (enemies > 0).then_some(enemies as u32);
        }
    }
    None
}

/// プレイヤーの指標を計算する。試合に参加していなければ `None`。
pub fn player_metrics(puuid: &str, match_id: &str, details: &Value, catalog: &GameCatalog) -> Option<MatchMetrics> {
    let players = list(details, "players");
    let player = players.iter().find(|p| text(p, "subject") == Some(puuid))?;
    let null = Value::Null;
    let stats = player.get("stats").unwrap_or(&null);
    let team = text(player, "teamId");
    let teams: HashMap<&str, &str> = players
        .iter()
        .filter_map(|p| Some((text(p, "subject")?, text(p, "teamId")?)))
        .collect();
    let attackers_first = first_half_attackers(details, &teams);
    let is_ally = |subject: Option<&str>| subject.and_then(|s| teams.get(s)).copied() == team && team.is_some();

    let rounds = list(details, "roundResults");
    let mut metrics = MatchMetrics {
        match_id: match_id.to_owned(),
        started_ms: details.get("matchInfo").map_or(0, |info| int(info, "gameStartMillis")),
        map: details
            .get("matchInfo")
            .and_then(|info| text(info, "mapId"))
            .and_then(|id| catalog.maps.get(&id.to_lowercase()))
            .cloned(),
        agent: text(player, "characterId")
            .and_then(|id| catalog.agents.get(&id.to_lowercase()))
            .cloned(),
        won: list(details, "teams")
            .iter()
            .find(|t| text(t, "teamId") == team)
            .and_then(|t| t.get("won"))
            .and_then(Value::as_bool),
        rounds: rounds.len() as u32,
        score: int(stats, "score").max(0) as u32,
        kills: int(stats, "kills").max(0) as u32,
        deaths: int(stats, "deaths").max(0) as u32,
        assists: int(stats, "assists").max(0) as u32,
        ..Default::default()
    };

    for (index, round) in rounds.iter().enumerate() {
        let kills = round_kills(round);
        let own_stats = list(round, "playerStats").iter().find(|p| text(p, "subject") == Some(puuid));
        let round_won = team.is_some() && text(round, "winningTeam") == team;

        metrics.damage_received += list(round, "playerStats")
            .iter()
            .flat_map(|player| list(player, "damage"))
            .filter(|damage| text(damage, "receiver") == Some(puuid))
            .map(|damage| int(damage, "damage"))
            .sum::<i64>();

        let mut own_kills = 0;
        if let Some(own) = own_stats {
            own_kills = list(own, "kills").len() as u32;
            for damage in list(own, "damage") {
                metrics.damage += int(damage, "damage");
                let head = int(damage, "headshots").max(0) as u32;
                metrics.headshots += head;
                metrics.hits += head
                    + int(damage, "bodyshots").max(0) as u32
                    + int(damage, "legshots").max(0) as u32;
            }
            metrics.credits_spent += own.get("economy").map_or(0, |economy| int(economy, "spent"));
            let loadout = own.get("economy").map_or(0, |economy| int(economy, "loadoutValue"));
            // 装備の情報がないラウンドは経済の集計に含めない。
            if own.get("economy").is_some() {
                let bracket = match loadout {
                    value if value < ECO_LOADOUT => &mut metrics.eco,
                    value if value < FULL_LOADOUT => &mut metrics.half_buy,
                    _ => &mut metrics.full_buy,
                };
                bracket.add(round_won);
            }
        }

        if own_kills >= 3 {
            metrics.multi_kills += 1;
        }
        if index == 0 || index == REGULATION_HALF {
            metrics.pistol.add(round_won);
        }
        if let (Some(first), Some(team)) = (attackers_first, team) {
            if index < REGULATION_ROUNDS {
                let attacking = (team == first) == (index < REGULATION_HALF);
                let side = if attacking { &mut metrics.attack } else { &mut metrics.defense };
                side.add(round_won);
            }
        }

        if let Some(opening) = kills.first() {
            metrics.first_kills += u32::from(text(opening, "killer") == Some(puuid));
            metrics.first_deaths += u32::from(text(opening, "victim") == Some(puuid));
        }

        let assisted = kills.iter().any(|kill| {
            list(kill, "assistants").iter().any(|assistant| assistant.as_str() == Some(puuid))
        });
        let death = kills.iter().find(|kill| text(kill, "victim") == Some(puuid));
        let traded = death.is_some_and(|death| {
            let killer = text(death, "killer");
            let died_at = kill_time(death);
            kills.iter().any(|kill| {
                text(kill, "victim") == killer
                    && is_ally(text(kill, "killer"))
                    && (died_at..=died_at.saturating_add(TRADE_WINDOW_MS)).contains(&kill_time(kill))
            })
        });
        metrics.kast_rounds += u32::from(own_kills > 0 || assisted || death.is_none() || traded);
        metrics.survived_rounds += u32::from(death.is_none());
        metrics.traded_deaths += u32::from(traded);
        if clutch_opponents(puuid, team, &teams, &kills).is_some() {
            metrics.clutch.add(round_won);
        }
    }
    Some(metrics)
}

/// 試合で観測した 1 人分のピック。個人を特定する値は持たない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPick {
    pub team: String,
    pub agent: String,
    pub won: bool,
    pub tier: u32,
}

/// 試合詳細からマップと全員のピックを取り出す。名前と puuid は読まない。
pub fn observed_picks(details: &Value, catalog: &GameCatalog) -> Option<(String, Vec<ObservedPick>)> {
    let map_id = text(details.get("matchInfo")?, "mapId")?;
    let map = catalog.maps.get(&map_id.to_lowercase())?.clone();
    let winners: Vec<&str> = list(details, "teams")
        .iter()
        .filter(|team| team.get("won").and_then(Value::as_bool) == Some(true))
        .filter_map(|team| text(team, "teamId"))
        .collect();
    let picks = list(details, "players")
        .iter()
        .filter_map(|player| {
            let team = text(player, "teamId")?;
            let agent = catalog.agents.get(&text(player, "characterId")?.to_lowercase())?;
            Some(ObservedPick {
                team: team.to_owned(),
                agent: agent.clone(),
                won: winners.contains(&team),
                tier: int(player, "competitiveTier").max(0) as u32,
            })
        })
        .collect();
    Some((map, picks))
}

/// マップやエージェントごとの成績。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GroupStats {
    pub name: String,
    pub games: u32,
    pub wins: u32,
    pub acs: Option<u32>,
    pub kast: Option<u32>,
    pub kd: Option<f64>,
    pub adr: Option<u32>,
    pub headshot_percent: Option<u32>,
    /// 攻撃と防衛のラウンド勝率。マップごとの得手不得手を見るのに使う。
    pub attack: RoundRecord,
    pub defense: RoundRecord,
}

/// マップとエージェントの組み合わせごとの成績。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PairStats {
    pub map: String,
    pub agent: String,
    pub games: u32,
    pub wins: u32,
    pub acs: Option<u32>,
}

/// グラフと時間帯の集計に使う、1 試合分の値。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GameLine {
    pub started_ms: i64,
    pub won: Option<bool>,
    pub acs: Option<u32>,
    pub kast: Option<u32>,
}

/// 直近の試合と、それより前の試合の比較。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Trend {
    pub recent_games: u32,
    pub recent_acs: Option<u32>,
    pub previous_acs: Option<u32>,
    pub recent_kast: Option<u32>,
    pub previous_kast: Option<u32>,
}

/// 複数の試合の集計。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub games: u32,
    pub wins: u32,
    pub rounds: u32,
    pub acs: Option<u32>,
    pub adr: Option<u32>,
    pub kast: Option<u32>,
    pub kd: Option<f64>,
    pub headshot_percent: Option<u32>,
    pub first_kill_percent: Option<u32>,
    pub first_death_percent: Option<u32>,
    pub multi_kills: u32,
    pub damage_delta: Option<i32>,
    pub econ: Option<u32>,
    pub first_kill_difference: i32,
    pub kda: Option<f64>,
    pub assists_per_round: Option<f64>,
    pub survival_percent: Option<u32>,
    pub trade_percent: Option<u32>,
    pub clutch: RoundRecord,
    pub after_win: RoundRecord,
    pub after_loss: RoundRecord,
    pub trend: Option<Trend>,
    pub lines: Vec<GameLine>,
    pub pistol: RoundRecord,
    pub eco: RoundRecord,
    pub half_buy: RoundRecord,
    pub full_buy: RoundRecord,
    pub attack: RoundRecord,
    pub defense: RoundRecord,
    pub maps: Vec<GroupStats>,
    pub agents: Vec<GroupStats>,
    /// 試合数の多い順。
    pub pairs: Vec<PairStats>,
}

fn ratio(kills: u32, deaths: u32) -> Option<f64> {
    (kills > 0 || deaths > 0).then(|| {
        let value = f64::from(kills) / f64::from(deaths.max(1));
        (value * 100.0).round() / 100.0
    })
}

fn total(games: &[&MatchMetrics]) -> MatchMetrics {
    let mut sum = MatchMetrics::default();
    for game in games {
        sum.rounds += game.rounds;
        sum.score += game.score;
        sum.kills += game.kills;
        sum.deaths += game.deaths;
        sum.assists += game.assists;
        sum.damage += game.damage;
        sum.damage_received += game.damage_received;
        sum.credits_spent += game.credits_spent;
        sum.survived_rounds += game.survived_rounds;
        sum.traded_deaths += game.traded_deaths;
        sum.clutch.merge(game.clutch);
        sum.headshots += game.headshots;
        sum.hits += game.hits;
        sum.kast_rounds += game.kast_rounds;
        sum.first_kills += game.first_kills;
        sum.first_deaths += game.first_deaths;
        sum.multi_kills += game.multi_kills;
        sum.pistol.merge(game.pistol);
        sum.eco.merge(game.eco);
        sum.half_buy.merge(game.half_buy);
        sum.full_buy.merge(game.full_buy);
        sum.attack.merge(game.attack);
        sum.defense.merge(game.defense);
    }
    sum
}

fn groups<F>(games: &[MatchMetrics], key: F) -> Vec<GroupStats>
where
    F: Fn(&MatchMetrics) -> Option<&String>,
{
    let mut grouped: BTreeMap<&str, Vec<&MatchMetrics>> = BTreeMap::new();
    for game in games {
        if let Some(name) = key(game) {
            grouped.entry(name).or_default().push(game);
        }
    }
    let mut stats: Vec<GroupStats> = grouped
        .into_iter()
        .map(|(name, entries)| {
            let sum = total(&entries);
            GroupStats {
                name: name.to_owned(),
                games: entries.len() as u32,
                wins: entries.iter().filter(|game| game.won == Some(true)).count() as u32,
                acs: sum.acs(),
                kast: sum.kast(),
                kd: ratio(sum.kills, sum.deaths),
                adr: sum.adr(),
                headshot_percent: sum.headshot_percent(),
                attack: sum.attack,
                defense: sum.defense,
            }
        })
        .collect();
    stats.sort_by(|a, b| b.games.cmp(&a.games).then(a.name.cmp(&b.name)));
    stats
}

fn pairs(games: &[MatchMetrics]) -> Vec<PairStats> {
    let mut grouped: BTreeMap<(&str, &str), Vec<&MatchMetrics>> = BTreeMap::new();
    for game in games {
        if let (Some(map), Some(agent)) = (&game.map, &game.agent) {
            grouped.entry((map, agent)).or_default().push(game);
        }
    }
    let mut stats: Vec<PairStats> = grouped
        .into_iter()
        .map(|((map, agent), entries)| PairStats {
            map: map.to_owned(),
            agent: agent.to_owned(),
            games: entries.len() as u32,
            wins: entries.iter().filter(|game| game.won == Some(true)).count() as u32,
            acs: total(&entries).acs(),
        })
        .collect();
    stats.sort_by(|a, b| b.games.cmp(&a.games).then(a.map.cmp(&b.map)).then(a.agent.cmp(&b.agent)));
    stats
}

/// 直前の試合の結果ごとに、次の試合の勝敗を数える。`games` は新しい順。
fn results_after(games: &[MatchMetrics]) -> (RoundRecord, RoundRecord) {
    let mut after_win = RoundRecord::default();
    let mut after_loss = RoundRecord::default();
    for pair in games.windows(2) {
        let (newer, older) = (&pair[0], &pair[1]);
        let (Some(result), Some(previous)) = (newer.won, older.won) else { continue };
        if previous {
            after_win.add(result);
        } else {
            after_loss.add(result);
        }
    }
    (after_win, after_loss)
}

fn trend(games: &[MatchMetrics]) -> Option<Trend> {
    if games.len() <= TREND_RECENT {
        return None;
    }
    let (recent, previous) = games.split_at(TREND_RECENT);
    let recent = total(&recent.iter().collect::<Vec<_>>());
    let previous = total(&previous.iter().collect::<Vec<_>>());
    Some(Trend {
        recent_games: TREND_RECENT as u32,
        recent_acs: recent.acs(),
        previous_acs: previous.acs(),
        recent_kast: recent.kast(),
        previous_kast: previous.kast(),
    })
}

/// 試合を集計する。`games` は新しい順に渡す。
pub fn summarize(games: &[MatchMetrics]) -> Summary {
    let refs: Vec<&MatchMetrics> = games.iter().collect();
    let sum = total(&refs);
    let (after_win, after_loss) = results_after(games);
    Summary {
        games: games.len() as u32,
        wins: games.iter().filter(|game| game.won == Some(true)).count() as u32,
        rounds: sum.rounds,
        acs: sum.acs(),
        adr: sum.adr(),
        kast: sum.kast(),
        kd: ratio(sum.kills, sum.deaths),
        headshot_percent: sum.headshot_percent(),
        first_kill_percent: sum.first_kill_percent(),
        first_death_percent: sum.first_death_percent(),
        multi_kills: sum.multi_kills,
        damage_delta: sum.damage_delta(),
        econ: sum.econ(),
        first_kill_difference: sum.first_kills as i32 - sum.first_deaths as i32,
        kda: (sum.kills + sum.assists > 0).then(|| {
            let value = f64::from(sum.kills + sum.assists) / f64::from(sum.deaths.max(1));
            (value * 100.0).round() / 100.0
        }),
        assists_per_round: (sum.rounds > 0).then(|| {
            (f64::from(sum.assists) * 100.0 / f64::from(sum.rounds)).round() / 100.0
        }),
        survival_percent: percent(sum.survived_rounds, sum.rounds),
        trade_percent: percent(sum.traded_deaths, sum.rounds - sum.survived_rounds.min(sum.rounds)),
        clutch: sum.clutch,
        after_win,
        after_loss,
        trend: trend(games),
        lines: games
            .iter()
            .map(|game| GameLine { started_ms: game.started_ms, won: game.won, acs: game.acs(), kast: game.kast() })
            .collect(),
        pistol: sum.pistol,
        eco: sum.eco,
        half_buy: sum.half_buy,
        full_buy: sum.full_buy,
        attack: sum.attack,
        defense: sum.defense,
        maps: groups(games, |game| game.map.as_ref()),
        agents: groups(games, |game| game.agent.as_ref()),
        pairs: pairs(games),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round(number: usize, winner: &str, kills: Value, damage: i64, loadout: i64) -> Value {
        json!({
            "roundNum": number, "winningTeam": winner,
            "playerStats": [
                {"subject": "me", "kills": [], "damage": [{"damage": damage, "headshots": 1, "bodyshots": 3, "legshots": 0}],
                 "economy": {"loadoutValue": loadout}},
                {"subject": "killer-list", "kills": kills}
            ]
        })
    }

    fn details(rounds: Vec<Value>) -> Value {
        json!({
            "matchInfo": {"mapId": "/m"},
            "players": [
                {"subject": "me", "teamId": "Blue", "characterId": "jett",
                 "stats": {"score": 1000, "kills": 4, "deaths": 2, "assists": 1}},
                {"subject": "mate", "teamId": "Blue", "stats": {}},
                {"subject": "foe", "teamId": "Red", "stats": {}}
            ],
            "teams": [{"teamId": "Blue", "won": true}],
            "roundResults": rounds
        })
    }

    fn kill(killer: &str, victim: &str, at: i64) -> Value {
        json!({"killer": killer, "victim": victim, "timeSinceRoundStartMillis": at})
    }

    fn metrics(rounds: Vec<Value>) -> MatchMetrics {
        let mut catalog = GameCatalog::default();
        catalog.maps.insert("/m".into(), "Ascent".into());
        catalog.agents.insert("jett".into(), "Jett".into());
        player_metrics("me", "m1", &details(rounds), &catalog).unwrap()
    }

    #[test]
    fn computes_per_round_values() {
        let m = metrics(vec![round(0, "Blue", json!([]), 100, 800), round(1, "Red", json!([]), 50, 4500)]);
        assert_eq!((m.acs(), m.adr()), (Some(500), Some(75)));
        assert_eq!(m.headshot_percent(), Some(25));
        assert_eq!(m.map.as_deref(), Some("Ascent"));
        assert_eq!(m.agent.as_deref(), Some("Jett"));
        assert_eq!(m.won, Some(true));
    }

    #[test]
    fn kast_counts_survival_and_excludes_untraded_deaths() {
        let survived = round(0, "Blue", json!([]), 0, 0);
        let died = round(1, "Red", json!([kill("foe", "me", 1000)]), 0, 0);
        assert_eq!(metrics(vec![survived, died]).kast(), Some(50));
    }

    #[test]
    fn kast_counts_a_trade_within_the_window_only() {
        let traded = round(0, "Red", json!([kill("foe", "me", 1000), kill("mate", "foe", 5900)]), 0, 0);
        assert_eq!(metrics(vec![traded]).kast(), Some(100));
        let late = round(0, "Red", json!([kill("foe", "me", 1000), kill("mate", "foe", 6100)]), 0, 0);
        assert_eq!(metrics(vec![late]).kast(), Some(0));
    }

    #[test]
    fn kast_counts_assists() {
        let mut assisted = kill("mate", "foe", 500);
        assisted["assistants"] = json!(["me"]);
        let r = round(0, "Red", json!([assisted, kill("foe", "me", 1000)]), 0, 0);
        assert_eq!(metrics(vec![r]).kast(), Some(100));
    }

    #[test]
    fn first_kill_uses_the_earliest_kill() {
        let r = round(0, "Red", json!([kill("foe", "me", 3000), kill("mate", "foe", 1000)]), 0, 0);
        let m = metrics(vec![r]);
        assert_eq!((m.first_kills, m.first_deaths), (0, 0));
        let r = round(0, "Red", json!([kill("foe", "me", 500)]), 0, 0);
        assert_eq!(metrics(vec![r]).first_deaths, 1);
    }

    #[test]
    fn splits_rounds_by_loadout_and_pistols() {
        let rounds: Vec<Value> = (0..14)
            .map(|n| {
                let loadout = match n { 0 | 12 => 800, 1 => 2500, _ => 4200 };
                round(n, if n % 2 == 0 { "Blue" } else { "Red" }, json!([]), 0, loadout)
            })
            .collect();
        let m = metrics(rounds);
        assert_eq!(m.pistol, RoundRecord { played: 2, won: 2 });
        assert_eq!(m.eco, RoundRecord { played: 2, won: 2 });
        assert_eq!(m.half_buy, RoundRecord { played: 1, won: 0 });
        assert_eq!(m.full_buy.played, 11);
    }

    #[test]
    fn missing_economy_is_not_counted_as_eco() {
        let r = json!({"winningTeam": "Blue", "playerStats": [{"subject": "me"}]});
        let m = metrics(vec![r]);
        assert_eq!(m.eco.played + m.half_buy.played + m.full_buy.played, 0);
    }

    #[test]
    fn sides_follow_the_bomb_planter_and_swap_at_halftime() {
        let mut rounds: Vec<Value> =
            (0..14).map(|n| round(n, "Blue", json!([]), 0, 0)).collect();
        rounds[2]["bombPlanter"] = json!("foe");
        let m = metrics(rounds);
        assert_eq!(m.defense.played, 12);
        assert_eq!(m.attack.played, 2);
    }

    #[test]
    fn a_second_half_plant_also_identifies_sides() {
        let mut rounds: Vec<Value> =
            (0..14).map(|n| round(n, "Blue", json!([]), 0, 0)).collect();
        rounds[13]["bombPlanter"] = json!("me");
        let m = metrics(rounds);
        assert_eq!((m.attack.played, m.defense.played), (2, 12));
    }

    #[test]
    fn sides_stay_empty_without_a_plant() {
        let m = metrics(vec![round(0, "Blue", json!([]), 0, 0)]);
        assert_eq!(m.attack.played + m.defense.played, 0);
    }

    #[test]
    fn unknown_player_and_empty_match_are_safe() {
        assert!(player_metrics("nobody", "m", &details(vec![]), &GameCatalog::default()).is_none());
        let m = metrics(vec![]);
        assert_eq!((m.acs(), m.kast()), (None, None));
    }

    #[test]
    fn summary_weights_by_rounds_and_groups_by_map() {
        let a = MatchMetrics {
            map: Some("Ascent".into()), agent: Some("Jett".into()), won: Some(true),
            rounds: 10, score: 3000, kills: 10, deaths: 5, damage: 1500, ..Default::default()
        };
        let b = MatchMetrics {
            map: Some("Bind".into()), agent: Some("Jett".into()), won: Some(false),
            rounds: 30, score: 3000, kills: 10, deaths: 15, damage: 1500, ..Default::default()
        };
        let summary = summarize(&[a, b]);
        assert_eq!((summary.games, summary.wins), (2, 1));
        assert_eq!(summary.acs, Some(150));
        assert_eq!(summary.adr, Some(75));
        assert_eq!(summary.kd, Some(1.0));
        assert_eq!(summary.agents[0].games, 2);
        assert_eq!(summary.maps.len(), 2);
    }

    fn team_details(rounds: Vec<Value>) -> Value {
        json!({
            "players": [
                {"subject": "me", "teamId": "Blue", "stats": {}},
                {"subject": "m2", "teamId": "Blue", "stats": {}},
                {"subject": "m3", "teamId": "Blue", "stats": {}},
                {"subject": "e1", "teamId": "Red", "stats": {}},
                {"subject": "e2", "teamId": "Red", "stats": {}}
            ],
            "roundResults": rounds
        })
    }

    #[test]
    fn counts_a_clutch_and_its_result() {
        let won = round(0, "Blue", json!([kill("e1", "m2", 1), kill("e1", "m3", 2), kill("x", "e1", 3)]), 0, 0);
        let lost = round(1, "Red", json!([kill("e1", "m2", 1), kill("e2", "m3", 2), kill("e1", "me", 3)]), 0, 0);
        let m = player_metrics("me", "m", &team_details(vec![won, lost]), &GameCatalog::default()).unwrap();
        assert_eq!(m.clutch, RoundRecord { played: 2, won: 1 });
    }

    #[test]
    fn dying_first_or_clearing_the_enemy_is_not_a_clutch() {
        let early = round(0, "Red", json!([kill("e1", "me", 1), kill("e1", "m2", 2)]), 0, 0);
        let cleared = round(1, "Blue", json!([kill("x", "e1", 1), kill("x", "e2", 2), kill("e1", "m2", 3), kill("e1", "m3", 4)]), 0, 0);
        let m = player_metrics("me", "m", &team_details(vec![early, cleared]), &GameCatalog::default()).unwrap();
        assert_eq!(m.clutch.played, 0);
    }

    #[test]
    fn damage_delta_and_econ() {
        let r = json!({"winningTeam": "Blue", "playerStats": [
            {"subject": "me", "damage": [{"damage": 200, "receiver": "foe"}], "economy": {"spent": 4000, "loadoutValue": 4000}},
            {"subject": "foe", "damage": [{"damage": 50, "receiver": "me"}, {"damage": 90, "receiver": "mate"}]}
        ]});
        let m = metrics(vec![r]);
        assert_eq!(m.damage_delta(), Some(150));
        assert_eq!(m.econ(), Some(50));
    }

    #[test]
    fn results_after_uses_the_previous_game() {
        let game = |won| MatchMetrics { won: Some(won), ..Default::default() };
        // 新しい順。古い順に読むと 負け、勝ち、勝ち、負け。
        let games = [game(false), game(true), game(true), game(false)];
        let summary = summarize(&games);
        assert_eq!(summary.after_loss, RoundRecord { played: 1, won: 1 });
        assert_eq!(summary.after_win, RoundRecord { played: 2, won: 1 });
    }

    #[test]
    fn trend_compares_recent_games_with_older_ones() {
        let game = |score| MatchMetrics { rounds: 10, score, ..Default::default() };
        let mut games: Vec<MatchMetrics> = (0..5).map(|_| game(3000)).collect();
        assert!(summarize(&games).trend.is_none(), "比べる過去がなければ出さない");
        games.extend((0..5).map(|_| game(2000)));
        let trend = summarize(&games).trend.unwrap();
        assert_eq!((trend.recent_acs, trend.previous_acs), (Some(300), Some(200)));
    }

    #[test]
    fn observed_picks_hold_no_identity() {
        let mut catalog = GameCatalog::default();
        catalog.maps.insert("/m".into(), "Ascent".into());
        catalog.agents.insert("jett".into(), "Jett".into());
        let value = details(vec![]);
        let (map, picks) = observed_picks(&value, &catalog).unwrap();
        assert_eq!(map, "Ascent");
        assert_eq!(picks, [ObservedPick { team: "Blue".into(), agent: "Jett".into(), won: true, tier: 0 }]);
    }

    #[test]
    fn no_deaths_does_not_divide_by_zero() {
        assert_eq!(ratio(5, 0), Some(5.0));
        assert_eq!(ratio(0, 0), None);
    }

    #[test]
    fn groups_carry_side_records_and_pairs_count_each_combination() {
        let game = |map: &str, agent: &str, won| MatchMetrics {
            map: Some(map.into()),
            agent: Some(agent.into()),
            won: Some(won),
            rounds: 10,
            score: 2000,
            damage: 1500,
            attack: RoundRecord { played: 5, won: 3 },
            defense: RoundRecord { played: 5, won: 1 },
            ..MatchMetrics::default()
        };
        let summary = summarize(&[game("Ascent", "Jett", true), game("Ascent", "Jett", false), game("Bind", "Sova", true)]);
        let ascent = summary.maps.iter().find(|group| group.name == "Ascent").unwrap();
        assert_eq!(ascent.attack, RoundRecord { played: 10, won: 6 });
        assert_eq!(ascent.defense.percent(), Some(20));
        assert_eq!(ascent.adr, Some(150));
        assert_eq!(summary.pairs.len(), 2);
        assert_eq!((summary.pairs[0].map.as_str(), summary.pairs[0].agent.as_str()), ("Ascent", "Jett"));
        assert_eq!((summary.pairs[0].games, summary.pairs[0].wins), (2, 1));
    }
}
