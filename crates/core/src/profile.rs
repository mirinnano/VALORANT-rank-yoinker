//! 自分のランクと RR の推移。試合の外でも見られる情報。

use crate::analytics::{MatchMetrics, Summary};
use crate::content::Seasons;
use crate::model::RankView;
use crate::rank;
use crate::source::GameCatalog;
use serde::Serialize;
use serde_json::Value;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankPoint {
    pub match_id: Option<String>,
    pub at_ms: i64,
    pub map: Option<String>,
    pub tier: u32,
    pub rank_name: &'static str,
    pub rr: u32,
    pub earned: i32,
    pub afk_penalty: i32,
}

/// 推移や集計の対象にする試合数。
pub const HISTORY_LIMIT: usize = 20;

/// 履歴に載る試合か。振り分け中など tier のない試合は含めない。
pub fn counts_in_history(entry: &Value) -> bool {
    entry.get("TierAfterUpdate").and_then(Value::as_i64).is_some_and(|tier| tier > 0)
}

/// 全モードの最近の試合。一覧の情報に、集計済みなら指標を足す。
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct RecentMatch {
    pub match_id: String,
    pub at_ms: i64,
    pub queue: Option<String>,
    pub mode: Option<String>,
    pub map: Option<String>,
    pub agent: Option<String>,
    pub won: Option<bool>,
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub acs: Option<u32>,
    /// ランク戦なら RR の増減。
    pub rr_earned: Option<i32>,
}

/// スコアボードのある通常の対戦か。指標の集計はこのモードだけにする。
pub fn has_scoreboard(queue: Option<&str>) -> bool {
    matches!(queue, Some("competitive" | "unrated" | "swiftplay" | "premier" | "newmap" | "spikerush"))
}

/// 試合一覧と RR の履歴から、新しい順の最近の試合を作る。
/// 一覧を取得できなければ、RR の履歴だけで作る。
pub fn recent_matches(list: Option<&Value>, history: &[RankPoint]) -> Vec<RecentMatch> {
    let ranked = |id: &str| history.iter().find(|point| point.match_id.as_deref() == Some(id));
    let listed: Vec<RecentMatch> = list
        .and_then(|value| value.get("History"))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let match_id = entry.get("MatchID")?.as_str()?.to_owned();
                    let queue = entry.get("QueueID").and_then(Value::as_str).map(str::to_owned);
                    let point = ranked(&match_id);
                    Some(RecentMatch {
                        at_ms: entry.get("GameStartTime").and_then(Value::as_i64).unwrap_or(0),
                        mode: crate::engine::mode_name(queue.as_deref()),
                        map: point.and_then(|p| p.map.clone()),
                        rr_earned: point.map(|p| p.earned),
                        queue,
                        match_id,
                        ..Default::default()
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut matches = if listed.is_empty() {
        history
            .iter()
            .filter_map(|point| {
                Some(RecentMatch {
                    match_id: point.match_id.clone()?,
                    at_ms: point.at_ms,
                    queue: Some("competitive".to_owned()),
                    mode: crate::engine::mode_name(Some("competitive")),
                    map: point.map.clone(),
                    rr_earned: Some(point.earned),
                    ..Default::default()
                })
            })
            .collect()
    } else {
        listed
    };
    matches.sort_by_key(|m| std::cmp::Reverse(m.at_ms));
    matches.truncate(HISTORY_LIMIT);
    matches
}

/// 集計済みの指標を最近の試合へ書き込む。
pub fn apply_metrics(matches: &mut [RecentMatch], metrics: &[MatchMetrics]) {
    for entry in matches {
        let Some(game) = metrics.iter().find(|game| game.match_id == entry.match_id) else { continue };
        entry.map = entry.map.take().or_else(|| game.map.clone());
        entry.agent = game.agent.clone();
        entry.won = game.won;
        entry.kills = Some(game.kills);
        entry.deaths = Some(game.deaths);
        entry.assists = Some(game.assists);
        entry.acs = game.score.checked_div(game.rounds);
        if entry.at_ms == 0 {
            entry.at_ms = game.started_ms;
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Profile {
    pub name: Option<String>,
    pub rank: RankView,
    pub peak: RankView,
    pub peak_act: Option<String>,
    pub win_rate: Option<u32>,
    pub games: u32,
    /// 直近 24 時間の RR の増減。
    pub last_day_rr: i32,
    pub last_day_games: u32,
    /// 古い順。グラフの左から右へ並ぶ。
    pub history: Vec<RankPoint>,
    /// 保存済みの試合の集計。
    pub summary: Option<Summary>,
    /// 集計の対象にした試合のうち、試合詳細まで集計できた試合数。
    pub detailed_games: u32,
    /// 全モードの最近の試合。新しい順。
    pub matches: Vec<RecentMatch>,
    /// 一部の取得に失敗し、あとで取り直す。
    pub partial: bool,
}

fn point(value: &Value, catalog: &GameCatalog) -> Option<RankPoint> {
    let at_ms = value.get("MatchStartTime")?.as_i64()?;
    let int = |key| value.get(key).and_then(Value::as_i64).unwrap_or(0);
    if !counts_in_history(value) {
        return None;
    }
    let tier = int("TierAfterUpdate") as u32;
    let map = value
        .get("MapID")
        .and_then(Value::as_str)
        .and_then(|id| catalog.maps.get(&id.to_lowercase()))
        .cloned();
    Some(RankPoint {
        match_id: value.get("MatchID").and_then(Value::as_str).map(str::to_owned),
        at_ms,
        map,
        tier,
        rank_name: rank::tier_name(tier),
        rr: int("RankedRatingAfterUpdate") as u32,
        earned: int("RankedRatingEarned") as i32,
        afk_penalty: int("AFKPenalty") as i32,
    })
}

pub fn build(
    name: Option<String>,
    mmr: Option<&Value>,
    history: Option<&Value>,
    match_list: Option<&Value>,
    seasons: &Seasons,
    catalog: &GameCatalog,
    now_ms: i64,
) -> Profile {
    let current = seasons.current_id();
    let season = rank::season_rank(mmr, current);
    let peak = rank::peak_rank(mmr, &season, current);

    let mut points: Vec<RankPoint> = history
        .and_then(|history| history.get("Matches"))
        .and_then(Value::as_array)
        .map(|matches| matches.iter().filter_map(|m| point(m, catalog)).collect())
        .unwrap_or_default();
    points.sort_by_key(|point| point.at_ms);
    let overflow = points.len().saturating_sub(HISTORY_LIMIT);
    points.drain(..overflow);

    let recent = || points.iter().filter(|point| now_ms - point.at_ms < DAY_MS);
    let (last_day_rr, last_day_games) = (recent().map(|p| p.earned).sum(), recent().count() as u32);

    Profile {
        name,
        rank: rank::view(season.tier, season.rr, season.leaderboard),
        peak: rank::view(peak.tier, 0, 0),
        peak_act: peak.season.as_deref().and_then(|id| seasons.act_label(id)),
        win_rate: season.win_rate,
        games: season.games,
        last_day_rr,
        last_day_games,
        matches: recent_matches(match_list, &points),
        history: points,
        summary: None,
        detailed_games: 0,
        partial: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seasons() -> Seasons {
        Seasons::from_content(&json!({"Seasons": [
            {"ID": "e", "Name": "EPISODE 9", "Type": "episode"},
            {"ID": "s", "Name": "ACT 1", "Type": "act", "IsActive": true}
        ]}))
        .unwrap()
    }

    fn entry(at: i64, tier: i64, earned: i64) -> Value {
        json!({"MatchStartTime": at, "TierAfterUpdate": tier, "RankedRatingAfterUpdate": 40,
               "RankedRatingEarned": earned, "MapID": "/Game/A"})
    }

    #[test]
    fn sorts_oldest_first_and_sums_the_last_day() {
        let now = DAY_MS * 10;
        let history = json!({"Matches": [
            entry(now - 1000, 12, 20),
            entry(now - DAY_MS - 1, 12, 99),
            entry(now - 2000, 12, -5),
        ]});
        let mut catalog = GameCatalog::default();
        catalog.maps.insert("/game/a".into(), "Ascent".into());
        let p = build(None, None, Some(&history), None, &seasons(), &catalog, now);
        let times: Vec<_> = p.history.iter().map(|point| point.at_ms).collect();
        assert_eq!(times, [now - DAY_MS - 1, now - 2000, now - 1000]);
        assert_eq!((p.last_day_rr, p.last_day_games), (15, 2));
        assert_eq!(p.history[0].map.as_deref(), Some("Ascent"));
        assert_eq!(p.history[0].rank_name, "Gold 1");
    }

    #[test]
    fn placement_matches_are_excluded() {
        let history = json!({"Matches": [entry(1, 0, 0), {"RankedRatingEarned": 5}]});
        let p = build(None, None, Some(&history), None, &seasons(), &GameCatalog::default(), 2);
        assert!(p.history.is_empty());
    }

    #[test]
    fn missing_history_is_empty() {
        let p = build(None, None, None, None, &seasons(), &GameCatalog::default(), 0);
        assert!(p.history.is_empty());
        assert_eq!(p.rank.tier, 0);
    }

    #[test]
    fn recent_matches_cover_every_mode_and_keep_rr() {
        let history = json!({"Matches": [
            {"MatchID": "r1", "MatchStartTime": 100, "TierAfterUpdate": 12, "RankedRatingEarned": 18, "MapID": "/Game/A"}
        ]});
        let list = json!({"History": [
            {"MatchID": "u1", "GameStartTime": 200, "QueueID": "unrated"},
            {"MatchID": "r1", "GameStartTime": 100, "QueueID": "competitive"},
        ]});
        let p = build(None, None, Some(&history), Some(&list), &seasons(), &GameCatalog::default(), 300);
        let ids: Vec<_> = p.matches.iter().map(|m| m.match_id.as_str()).collect();
        assert_eq!(ids, ["u1", "r1"]);
        assert_eq!(p.matches[0].mode.as_deref(), Some("Unrated"));
        assert_eq!(p.matches[1].rr_earned, Some(18));
    }

    #[test]
    fn recent_matches_fall_back_to_rank_history() {
        let history = json!({"Matches": [entry(5, 12, 7)]});
        let p = build(None, None, Some(&history), None, &seasons(), &GameCatalog::default(), 10);
        assert_eq!(p.matches.len(), 0, "MatchID のない履歴は一覧にしない");
        let history = json!({"Matches": [{"MatchID": "x", "MatchStartTime": 5, "TierAfterUpdate": 12, "RankedRatingEarned": 7}]});
        let p = build(None, None, Some(&history), None, &seasons(), &GameCatalog::default(), 10);
        assert_eq!(p.matches[0].queue.as_deref(), Some("competitive"));
    }

    #[test]
    fn scoreboard_modes_exclude_deathmatch() {
        assert!(has_scoreboard(Some("unrated")));
        assert!(!has_scoreboard(Some("deathmatch")));
        assert!(!has_scoreboard(None));
    }
}
