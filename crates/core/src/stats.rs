//! 直近のコンペティティブ試合から成績を求める。

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecentStats {
    pub kd: Option<f64>,
    pub headshot_percent: Option<u32>,
    pub earned_rr: Option<i32>,
    pub afk_penalty: Option<i32>,
    pub last_active_ms: Option<i64>,
    /// 直近の試合の RR 増減。新しい順。
    pub form: Vec<i32>,
}

/// `competitiveupdates` の応答から直近の試合を取り出す。
pub fn latest_match(updates: &Value) -> Option<&Value> {
    updates.get("Matches")?.as_array()?.first()
}

/// 直近の試合の RR 増減を返す。勝ち負けの印に使う。
pub fn recent_form(updates: &Value) -> Vec<i32> {
    updates
        .get("Matches")
        .and_then(Value::as_array)
        .map(|matches| {
            matches
                .iter()
                .filter_map(|entry| entry.get("RankedRatingEarned")?.as_i64())
                .map(|earned| earned as i32)
                .collect()
        })
        .unwrap_or_default()
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn int(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

pub fn from_match(puuid: &str, summary: &Value, details: Option<&Value>) -> RecentStats {
    let null = Value::Null;
    let details = details.unwrap_or(&null);

    let (mut hits, mut headshots) = (0, 0);
    let rounds = list(details, "roundResults");
    let round_players = rounds.iter().flat_map(|round| list(round, "playerStats"));
    for player in round_players.filter(|p| p.get("subject").and_then(Value::as_str) == Some(puuid)) {
        for damage in list(player, "damage") {
            let head = int(damage, "headshots").unwrap_or(0);
            headshots += head;
            hits += head + int(damage, "bodyshots").unwrap_or(0) + int(damage, "legshots").unwrap_or(0);
        }
    }

    let kd = list(details, "players")
        .iter()
        .find(|p| p.get("subject").and_then(Value::as_str) == Some(puuid))
        .and_then(|p| p.get("stats"))
        .map(|stats| {
            let kills = int(stats, "kills").unwrap_or(0) as f64;
            let deaths = int(stats, "deaths").unwrap_or(0) as f64;
            let ratio = if deaths > 0.0 { kills / deaths } else { kills };
            (ratio * 100.0).round() / 100.0
        });

    let info = details.get("matchInfo").unwrap_or(&null);
    let started = int(summary, "MatchStartTime").or_else(|| int(info, "gameStartMillis"));
    let length = int(info, "gameLengthMillis").unwrap_or(0);

    RecentStats {
        kd,
        headshot_percent: (hits > 0).then(|| (headshots as f64 * 100.0 / hits as f64).round() as u32),
        earned_rr: int(summary, "RankedRatingEarned").map(|n| n as i32),
        afk_penalty: int(summary, "AFKPenalty").map(|n| n as i32),
        last_active_ms: started.map(|start| start + length),
        form: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn details() -> Value {
        json!({
            "matchInfo": {"gameLengthMillis": 1000},
            "players": [
                {"subject": "me", "stats": {"kills": 21, "deaths": 14}},
                {"subject": "zero", "stats": {"kills": 5, "deaths": 0}}
            ],
            "roundResults": [
                {"playerStats": [
                    {"subject": "me", "damage": [{"headshots": 2, "bodyshots": 5, "legshots": 1}]},
                    {"subject": "other", "damage": [{"headshots": 99, "bodyshots": 0, "legshots": 0}]}
                ]},
                {"playerStats": [
                    {"subject": "me", "damage": [{"headshots": 1, "bodyshots": 1, "legshots": 0}]}
                ]}
            ]
        })
    }

    #[test]
    fn computes_stats_for_the_requested_player_only() {
        let summary = json!({"RankedRatingEarned": -14, "AFKPenalty": 0, "MatchStartTime": 5000});
        let s = from_match("me", &summary, Some(&details()));
        assert_eq!(s.kd, Some(1.5));
        assert_eq!(s.headshot_percent, Some(30));
        assert_eq!(s.earned_rr, Some(-14));
        assert_eq!(s.afk_penalty, Some(0));
        assert_eq!(s.last_active_ms, Some(6000));
    }

    #[test]
    fn zero_deaths_uses_kills() {
        assert_eq!(from_match("zero", &json!({}), Some(&details())).kd, Some(5.0));
    }

    #[test]
    fn missing_details_leave_gaps() {
        let s = from_match("me", &json!({"RankedRatingEarned": 10}), None);
        assert_eq!(s.kd, None);
        assert_eq!(s.headshot_percent, None);
        assert_eq!(s.earned_rr, Some(10));
    }

    #[test]
    fn recent_form_keeps_order_and_skips_gaps() {
        let updates = json!({"Matches": [
            {"RankedRatingEarned": 18}, {}, {"RankedRatingEarned": -7}, {"RankedRatingEarned": 0}
        ]});
        assert_eq!(recent_form(&updates), [18, -7, 0]);
        assert!(recent_form(&json!({})).is_empty());
    }

    #[test]
    fn latest_match_handles_empty_list() {
        assert!(latest_match(&json!({"Matches": []})).is_none());
        assert!(latest_match(&json!({})).is_none());
    }
}
