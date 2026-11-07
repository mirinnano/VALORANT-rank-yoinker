//! MMR 応答からランクを算出する。

use crate::model::RankView;
use serde_json::Value;

pub const TIER_NAMES: [&str; 28] = [
    "Unranked", "Unranked", "Unranked", "Iron 1", "Iron 2", "Iron 3", "Bronze 1", "Bronze 2",
    "Bronze 3", "Silver 1", "Silver 2", "Silver 3", "Gold 1", "Gold 2", "Gold 3", "Platinum 1",
    "Platinum 2", "Platinum 3", "Diamond 1", "Diamond 2", "Diamond 3", "Ascendant 1",
    "Ascendant 2", "Ascendant 3", "Immortal 1", "Immortal 2", "Immortal 3", "Radiant",
];

/// Ascendant 追加前のシーズン。ここでは旧 Immortal 以上の tier を 3 つ上へずらす。
pub const BEFORE_ASCENDANT_SEASONS: [&str; 17] = [
    "0df5adb9-4dcb-6899-1306-3e9860661dd3", "3f61c772-4560-cd3f-5d3f-a7ab5abda6b3",
    "0530b9c4-4980-f2ee-df5d-09864cd00542", "46ea6166-4573-1128-9cea-60a15640059b",
    "fcf2c8f4-4324-e50b-2e23-718e4a3ab046", "97b6e739-44cc-ffa7-49ad-398ba502ceb0",
    "ab57ef51-4e59-da91-cc8d-51a5a2b9b8ff", "52e9749a-429b-7060-99fe-4595426a0cf7",
    "71c81c67-4fae-ceb1-844c-aab2bb8710fa", "2a27e5d2-4d30-c9e2-b15a-93b8909a442c",
    "4cb622e1-4244-6da3-7276-8daaf1c01be2", "a16955a5-4ad0-f761-5e9e-389df1c892fb",
    "97b39124-46ce-8b55-8fd1-7cbf7ffe173f", "573f53ac-41a5-3a7d-d9ce-d6a6298e5704",
    "d929bc38-4ab6-7da4-94f0-ee84f8ac141e", "3e47230a-463c-a301-eb7d-67bb60357d4f",
    "808202d6-4f2b-a8ff-1feb-b3a0590ad79f",
];

pub fn tier_name(tier: u32) -> &'static str {
    TIER_NAMES.get(tier as usize).copied().unwrap_or("Unknown")
}

pub fn view(tier: u32, rr: u32, leaderboard: u32) -> RankView {
    RankView { tier, name: tier_name(tier), rr, leaderboard }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SeasonRank {
    pub tier: u32,
    pub rr: u32,
    pub leaderboard: u32,
    pub games: u32,
    pub win_rate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Peak {
    pub tier: u32,
    pub season: Option<String>,
}

fn seasons(mmr: &Value) -> Option<&serde_json::Map<String, Value>> {
    mmr.get("QueueSkills")?.get("competitive")?.get("SeasonalInfoBySeasonID")?.as_object()
}

fn number(value: &Value, key: &str) -> u32 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0) as u32
}

/// 指定シーズンのランク。データがなければ既定値を返す。
pub fn season_rank(mmr: Option<&Value>, season_id: Option<&str>) -> SeasonRank {
    let info = mmr.and_then(seasons).zip(season_id).and_then(|(all, id)| all.get(id));
    let Some(info) = info else { return SeasonRank::default() };

    let tier = number(info, "CompetitiveTier");
    let games = number(info, "NumberOfGames");
    let wins = number(info, "NumberOfWinsWithPlacements");
    let (tier, rr, leaderboard) = match tier {
        0..=2 => (0, 0, 0),
        3..=20 => (tier, number(info, "RankedRating"), 0),
        _ => (tier, number(info, "RankedRating"), number(info, "LeaderboardRank")),
    };
    // 元実装は試合数 0 を勝率 100% と表示していた。
    let win_rate = (games > 0).then(|| wins * 100 / games);
    SeasonRank { tier, rr, leaderboard, games, win_rate }
}

/// 全シーズンの勝利履歴から最高ランクを求める。
pub fn peak_rank(mmr: Option<&Value>, current: &SeasonRank, current_season: Option<&str>) -> Peak {
    let mut peak = Peak { tier: current.tier, season: current_season.map(str::to_owned) };
    let Some(all) = mmr.and_then(seasons) else { return peak };

    for (season, info) in all {
        let Some(wins) = info.get("WinsByTier").and_then(Value::as_object) else { continue };
        for tier in wins.keys().filter_map(|tier| tier.parse::<u32>().ok()) {
            let before_ascendant = BEFORE_ASCENDANT_SEASONS.contains(&season.as_str());
            let tier = if before_ascendant && tier > 20 { tier + 3 } else { tier };
            if tier > peak.tier {
                peak = Peak { tier, season: Some(season.clone()) };
            }
        }
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OLD: &str = "0df5adb9-4dcb-6899-1306-3e9860661dd3";

    fn mmr(seasons: Value) -> Value {
        json!({"QueueSkills": {"competitive": {"SeasonalInfoBySeasonID": seasons}}})
    }

    #[test]
    fn tier_table_matches_original() {
        assert_eq!(tier_name(0), "Unranked");
        assert_eq!(tier_name(3), "Iron 1");
        assert_eq!(tier_name(21), "Ascendant 1");
        assert_eq!(tier_name(27), "Radiant");
        assert_eq!(tier_name(99), "Unknown");
    }

    #[test]
    fn reads_season() {
        let m = mmr(json!({"s": {
            "CompetitiveTier": 15, "RankedRating": 42, "LeaderboardRank": 7,
            "NumberOfGames": 8, "NumberOfWinsWithPlacements": 5
        }}));
        let r = season_rank(Some(&m), Some("s"));
        assert_eq!((r.tier, r.rr, r.games, r.win_rate), (15, 42, 8, Some(62)));
        // leaderboard は Ascendant 未満では無視する。
        assert_eq!(r.leaderboard, 0);
    }

    #[test]
    fn keeps_leaderboard_for_high_ranks() {
        let m = mmr(json!({"s": {"CompetitiveTier": 27, "RankedRating": 500, "LeaderboardRank": 12}}));
        assert_eq!(season_rank(Some(&m), Some("s")).leaderboard, 12);
    }

    #[test]
    fn low_tiers_are_unranked() {
        let m = mmr(json!({"s": {"CompetitiveTier": 2, "RankedRating": 50}}));
        let r = season_rank(Some(&m), Some("s"));
        assert_eq!((r.tier, r.rr), (0, 0));
    }

    // 元実装はこの失敗時に未定義変数の参照で例外になっていた。
    #[test]
    fn missing_response_does_not_panic() {
        assert_eq!(season_rank(None, Some("s")), SeasonRank::default());
        assert_eq!(peak_rank(None, &SeasonRank::default(), Some("s")).tier, 0);
    }

    #[test]
    fn zero_games_has_no_win_rate() {
        let m = mmr(json!({"s": {"CompetitiveTier": 10, "NumberOfGames": 0}}));
        assert_eq!(season_rank(Some(&m), Some("s")).win_rate, None);
    }

    #[test]
    fn peak_shifts_only_old_seasons() {
        let m = mmr(json!({
            OLD: {"WinsByTier": {"21": 3}},
            "new": {"WinsByTier": {"22": 1, "bad": 1}},
            "empty": {"WinsByTier": null}
        }));
        let p = peak_rank(Some(&m), &SeasonRank::default(), Some("new"));
        assert_eq!((p.tier, p.season.as_deref()), (24, Some(OLD)));
    }

    #[test]
    fn peak_keeps_current_when_history_is_lower() {
        let m = mmr(json!({"a": {"WinsByTier": {"5": 1}}}));
        let current = SeasonRank { tier: 12, ..Default::default() };
        let p = peak_rank(Some(&m), &current, Some("cur"));
        assert_eq!((p.tier, p.season.as_deref()), (12, Some("cur")));
    }
}
