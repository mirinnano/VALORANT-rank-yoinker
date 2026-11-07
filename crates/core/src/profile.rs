//! 自分のランクと RR の推移。試合の外でも見られる情報。

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
}

fn point(value: &Value, catalog: &GameCatalog) -> Option<RankPoint> {
    let at_ms = value.get("MatchStartTime")?.as_i64()?;
    let int = |key| value.get(key).and_then(Value::as_i64).unwrap_or(0);
    // 振り分け中など tier のない試合は推移に含めない。
    let tier = u32::try_from(int("TierAfterUpdate")).ok().filter(|tier| *tier > 0)?;
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
        history: points,
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
        let p = build(None, None, Some(&history), &seasons(), &catalog, now);
        let times: Vec<_> = p.history.iter().map(|point| point.at_ms).collect();
        assert_eq!(times, [now - DAY_MS - 1, now - 2000, now - 1000]);
        assert_eq!((p.last_day_rr, p.last_day_games), (15, 2));
        assert_eq!(p.history[0].map.as_deref(), Some("Ascent"));
        assert_eq!(p.history[0].rank_name, "Gold 1");
    }

    #[test]
    fn placement_matches_are_excluded() {
        let history = json!({"Matches": [entry(1, 0, 0), {"RankedRatingEarned": 5}]});
        let p = build(None, None, Some(&history), &seasons(), &GameCatalog::default(), 2);
        assert!(p.history.is_empty());
    }

    #[test]
    fn missing_history_is_empty() {
        let p = build(None, None, None, &seasons(), &GameCatalog::default(), 0);
        assert!(p.history.is_empty());
        assert_eq!(p.rank.tier, 0);
    }
}
