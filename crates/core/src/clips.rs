//! 録画からクリップを切り出す範囲を決める。
//!
//! 試合中の出来事はリアルタイムには取れないので、試合後の試合詳細にある時刻を使う。
//! キルの時刻は `gameStartMillis` からの経過で記録されている。これを録画を始めた時刻と
//! 比べて、動画の中の位置にする。Riot のサーバーと手元の時計はずれることがあるので、
//! 録画ごとに `offset_ms` で合わせ直せるようにする。

use crate::source::GameCatalog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 続けて倒したとみなす、キルとキルの間隔。
const STREAK_GAP_MS: i64 = 10_000;
/// 最初のキルの前に含める時間。
const LEAD_MS: i64 = 7_000;
/// 最後のキルの後に含める時間。
const TAIL_MS: i64 = 3_000;
/// ホットキーで残す、押す前の時間。
pub const MANUAL_BEFORE_MS: i64 = 30_000;
/// ホットキーで残す、押した後の時間。
pub const MANUAL_AFTER_MS: i64 = 2_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    /// 自分のキル。続けて倒した分はまとめる。
    Kills,
    /// 自分だけが残って勝ったラウンド。
    Clutch,
    /// 自分がスパイクを解除した場面。
    Defuse,
    /// ホットキーで残した場面。
    Manual,
}

impl ClipKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kills => "kills",
            Self::Clutch => "clutch",
            Self::Defuse => "defuse",
            Self::Manual => "manual",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "kills" => Self::Kills,
            "clutch" => Self::Clutch,
            "defuse" => Self::Defuse,
            "manual" => Self::Manual,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Kill,
    Defuse,
}

/// クリップの中の見せ場。共有用の書き出しで、キルフィードのように出す。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipEvent {
    pub at_ms: i64,
    pub kind: EventKind,
    pub weapon: Option<String>,
    /// 倒した相手のエージェント。名前は持たない。
    pub victim: Option<String>,
}

/// 切り出す範囲。時刻はすべて手元の時計のエポックミリ秒。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipPlan {
    pub kind: ClipKind,
    pub round: Option<u32>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub kills: u32,
    pub title: String,
    /// 見せ場。キルや解除の瞬間。
    pub events: Vec<ClipEvent>,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn game_time(event: &Value) -> Option<i64> {
    event.get("timeSinceGameStartMillis").or_else(|| event.get("gameTime")).and_then(Value::as_i64)
}

fn round_time(event: &Value) -> Option<i64> {
    event.get("timeSinceRoundStartMillis").or_else(|| event.get("roundTime")).and_then(Value::as_i64)
}

fn streak_title(kills: u32) -> String {
    match kills {
        5.. => "Ace".to_owned(),
        2..=4 => format!("{kills}K"),
        _ => "キル".to_owned(),
    }
}

/// 試合詳細から、自分の見せ場を切り出す範囲を作る。時刻は Riot 側の時計のまま返す。
pub fn plan(details: &Value, own: &str, catalog: &GameCatalog) -> Vec<ClipPlan> {
    let Some(game_start) = details.pointer("/matchInfo/gameStartMillis").and_then(Value::as_i64) else {
        return Vec::new();
    };
    let players = list(details, "players");
    let team = players.iter().find(|p| text(p, "subject") == Some(own)).and_then(|p| text(p, "teamId"));
    let allies: HashSet<&str> = players
        .iter()
        .filter(|p| team.is_some() && text(p, "teamId") == team)
        .filter_map(|p| text(p, "subject"))
        .collect();

    let agents: HashMap<&str, String> = players
        .iter()
        .filter_map(|p| Some((text(p, "subject")?, catalog.agents.get(&text(p, "characterId")?.to_lowercase())?.clone())))
        .collect();
    let kill_event = |kill: &Value| -> Option<ClipEvent> {
        Some(ClipEvent {
            at_ms: game_start + game_time(kill)?,
            kind: EventKind::Kill,
            weapon: crate::review::weapon_name(kill, catalog),
            victim: text(kill, "victim").and_then(|v| agents.get(v)).cloned(),
        })
    };
    let defuse_event = |at_ms| ClipEvent { at_ms, kind: EventKind::Defuse, weapon: None, victim: None };

    let mut plans = Vec::new();
    for (index, round) in list(details, "roundResults").iter().enumerate() {
        let number = index as u32 + 1;
        let mut kills: Vec<&Value> =
            list(round, "playerStats").iter().flat_map(|player| list(player, "kills")).collect();
        kills.sort_by_key(|kill| game_time(kill).unwrap_or(i64::MAX));
        // ラウンドの開始時刻。キルの時刻から逆算する。
        let round_start = kills.iter().find_map(|kill| Some(game_time(kill)? - round_time(kill)?));

        let own_kills: Vec<&Value> =
            kills.iter().copied().filter(|kill| text(kill, "killer") == Some(own) && game_time(kill).is_some()).collect();
        let own_times: Vec<i64> = own_kills.iter().filter_map(|kill| game_time(kill)).collect();
        let mut streak: Vec<&Value> = Vec::new();
        let mut flush = |streak: &mut Vec<&Value>| {
            let times: Vec<i64> = streak.iter().filter_map(|kill| game_time(kill)).collect();
            if let (Some(first), Some(last)) = (times.first(), times.last()) {
                plans.push(ClipPlan {
                    kind: ClipKind::Kills,
                    round: Some(number),
                    start_ms: game_start + first - LEAD_MS,
                    end_ms: game_start + last + TAIL_MS,
                    kills: times.len() as u32,
                    title: streak_title(times.len() as u32),
                    events: streak.iter().filter_map(|kill| kill_event(kill)).collect(),
                });
            }
            streak.clear();
        };
        for kill in own_kills.iter().copied() {
            let time = game_time(kill).unwrap_or_default();
            if streak.last().and_then(|last| game_time(last)).is_some_and(|last| time - last > STREAK_GAP_MS) {
                flush(&mut streak);
            }
            streak.push(kill);
        }
        flush(&mut streak);

        let won = team.is_some() && text(round, "winningTeam") == team;
        // 自分以外の味方が全員倒れた後に勝っていれば、クラッチとみなす。
        let ally_deaths: Vec<i64> = kills
            .iter()
            .filter(|kill| text(kill, "victim").is_some_and(|v| v != own && allies.contains(v)))
            .filter_map(|kill| game_time(kill))
            .collect();
        let own_died = kills.iter().any(|kill| text(kill, "victim") == Some(own));
        let ally_count = list(round, "playerStats")
            .iter()
            .filter_map(|p| text(p, "subject"))
            .filter(|s| *s != own && allies.contains(s))
            .count();
        if won && !own_died && ally_count > 0 && ally_deaths.len() == ally_count {
            let alone_from = ally_deaths.iter().copied().max().unwrap_or(0);
            let end = own_times.iter().copied().max().unwrap_or(alone_from).max(alone_from);
            let defuse = round_start.zip(round.get("defuseRoundTime").and_then(Value::as_i64)).map(|(s, d)| s + d);
            let end = defuse.map_or(end, |d| d.max(end));
            let mut events: Vec<ClipEvent> = own_kills
                .iter()
                .filter(|kill| game_time(kill).is_some_and(|t| t >= alone_from))
                .filter_map(|kill| kill_event(kill))
                .collect();
            events.extend(defuse.map(|d| defuse_event(game_start + d)));
            plans.push(ClipPlan {
                kind: ClipKind::Clutch,
                round: Some(number),
                start_ms: game_start + alone_from - 2_000,
                end_ms: game_start + end + TAIL_MS,
                kills: own_times.iter().filter(|t| **t >= alone_from).count() as u32,
                title: "Clutch".to_owned(),
                events,
            });
        }

        if text(round, "bombDefuser") == Some(own) {
            if let (Some(start), Some(at)) = (round_start, round.get("defuseRoundTime").and_then(Value::as_i64)) {
                plans.push(ClipPlan {
                    kind: ClipKind::Defuse,
                    round: Some(number),
                    start_ms: game_start + start + at - 10_000,
                    end_ms: game_start + start + at + TAIL_MS,
                    kills: 0,
                    title: "Defuse".to_owned(),
                    events: vec![defuse_event(game_start + start + at)],
                });
            }
        }
    }
    plans
}

/// ホットキーを押した時刻から、残す範囲を作る。
pub fn manual(at_ms: i64) -> ClipPlan {
    ClipPlan {
        kind: ClipKind::Manual,
        round: None,
        start_ms: at_ms - MANUAL_BEFORE_MS,
        end_ms: at_ms + MANUAL_AFTER_MS,
        kills: 0,
        title: "保存した場面".to_owned(),
        events: Vec::new(),
    }
}

/// 録画 1 本の情報。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Recording {
    pub id: i64,
    pub match_id: Option<String>,
    pub path: String,
    /// 最初のフレームを受け取った時刻。手元の時計。
    pub started_ms: i64,
    pub length_ms: i64,
    pub bytes: u64,
    /// Riot 側の時刻へ足して、手元の時計に合わせる値。
    pub offset_ms: i64,
    pub map: Option<String>,
    pub mode: Option<String>,
    /// 録画の画面の大きさ。
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// 保存済みのクリップ。`start_ms` と `end_ms` は録画の先頭からの位置。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Clip {
    pub id: i64,
    pub recording_id: i64,
    pub kind: ClipKind,
    pub round: Option<u32>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub kills: u32,
    pub title: String,
    pub exported_path: Option<String>,
    /// 見せ場。時刻はクリップの先頭からのミリ秒。
    pub events: Vec<ClipEvent>,
}

/// 録画の中での位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub start_ms: i64,
    pub end_ms: i64,
    /// 時刻はクリップの先頭からのミリ秒。範囲の外の見せ場は除く。
    pub events: Vec<ClipEvent>,
}

/// 範囲を録画の中の位置へ直す。録画の外にはみ出す分は削り、重ならなければ `None`。
/// `riot_clock` は Riot 側の時刻で作った範囲か。ホットキーの範囲は手元の時計なので補正しない。
pub fn place(plan: &ClipPlan, recording: &Recording, riot_clock: bool) -> Option<Placement> {
    let shift = if riot_clock { recording.offset_ms } else { 0 };
    let start = (plan.start_ms + shift - recording.started_ms).max(0);
    let end = (plan.end_ms + shift - recording.started_ms).min(recording.length_ms);
    let events = plan
        .events
        .iter()
        .map(|event| ClipEvent { at_ms: event.at_ms + shift - recording.started_ms - start, ..event.clone() })
        .filter(|event| (0..=end - start).contains(&event.at_ms))
        .collect();
    (end - start >= 1_000).then_some(Placement { start_ms: start, end_ms: end, events })
}

/// 容量の上限を超えた古い録画を選ぶ。新しい `keep` 本と、上限内に収まる分は残す。
pub fn prune(recordings: &[Recording], keep: usize, max_bytes: u64) -> Vec<i64> {
    let mut newest: Vec<&Recording> = recordings.iter().collect();
    newest.sort_by_key(|r| std::cmp::Reverse(r.started_ms));
    let mut total = 0u64;
    newest
        .into_iter()
        .enumerate()
        .filter_map(|(index, recording)| {
            total = total.saturating_add(recording.bytes);
            // 一番新しい録画は、上限を超えていても消さない。
            (index > 0 && (index >= keep || total > max_bytes)).then_some(recording.id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kill(killer: &str, victim: &str, game: i64, round: i64) -> Value {
        json!({"killer": killer, "victim": victim, "timeSinceGameStartMillis": game, "timeSinceRoundStartMillis": round})
    }

    fn details(rounds: Vec<Value>) -> Value {
        json!({
            "matchInfo": {"gameStartMillis": 1_000_000},
            "players": [
                {"subject": "me", "teamId": "Blue"}, {"subject": "mate", "teamId": "Blue"},
                {"subject": "e1", "teamId": "Red"}, {"subject": "e2", "teamId": "Red"}
            ],
            "roundResults": rounds
        })
    }

    #[test]
    fn close_kills_become_one_clip() {
        let round = json!({"winningTeam": "Blue", "playerStats": [
            {"subject": "me", "kills": [kill("me", "e1", 60_000, 20_000), kill("me", "e2", 66_000, 26_000)]},
            {"subject": "mate", "kills": []}
        ]});
        let plans = plan(&details(vec![round]), "me", &GameCatalog::default());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].title, "2K");
        assert_eq!(plans[0].start_ms, 1_000_000 + 60_000 - LEAD_MS);
        assert_eq!(plans[0].end_ms, 1_000_000 + 66_000 + TAIL_MS);
        let times: Vec<_> = plans[0].events.iter().map(|e| e.at_ms).collect();
        assert_eq!(times, [1_060_000, 1_066_000]);
    }

    #[test]
    fn distant_kills_are_separate() {
        let round = json!({"winningTeam": "Red", "playerStats": [
            {"subject": "me", "kills": [kill("me", "e1", 60_000, 20_000), kill("me", "e2", 90_000, 50_000)]}
        ]});
        let titles: Vec<_> = plan(&details(vec![round]), "me", &GameCatalog::default()).into_iter().map(|p| p.title).collect();
        assert_eq!(titles, ["キル", "キル"]);
    }

    #[test]
    fn last_one_standing_win_is_a_clutch() {
        let round = json!({"winningTeam": "Blue", "bombDefuser": "me", "defuseRoundTime": 80_000,
            "playerStats": [
                {"subject": "me", "kills": [kill("me", "e1", 70_000, 30_000)]},
                {"subject": "mate", "kills": []},
                {"subject": "e2", "kills": [kill("e2", "mate", 50_000, 10_000)]}
            ]});
        let plans = plan(&details(vec![round]), "me", &GameCatalog::default());
        let clutch = plans.iter().find(|p| p.kind == ClipKind::Clutch).expect("clutch");
        assert_eq!(clutch.start_ms, 1_000_000 + 50_000 - 2_000);
        // 解除は ラウンド開始 40 秒 + 80 秒 の時点。
        assert_eq!(clutch.end_ms, 1_000_000 + 120_000 + TAIL_MS);
        assert!(plans.iter().any(|p| p.kind == ClipKind::Defuse));
    }

    #[test]
    fn missing_game_start_plans_nothing() {
        assert!(plan(&json!({"roundResults": []}), "me", &GameCatalog::default()).is_empty());
    }

    fn recording(id: i64, started: i64, bytes: u64) -> Recording {
        Recording {
            id, match_id: None, path: String::new(), started_ms: started, length_ms: 600_000, bytes, offset_ms: 0,
            map: None, mode: None, width: None, height: None,
        }
    }

    #[test]
    fn place_clamps_to_the_recording_and_applies_the_offset() {
        let mut rec = recording(1, 1_000, 0);
        let plan = ClipPlan {
            kind: ClipKind::Kills, round: Some(1), start_ms: 500, end_ms: 9_000, kills: 1, title: String::new(),
            events: [6_000, 50_000].map(|at_ms| ClipEvent { at_ms, kind: EventKind::Kill, weapon: None, victim: None }).to_vec(),
        };
        let times = |p: Option<Placement>| p.unwrap().events.iter().map(|e| e.at_ms).collect::<Vec<_>>();
        let at = |p: Option<Placement>| p.map(|p| (p.start_ms, p.end_ms));
        assert_eq!(at(place(&plan, &rec, true)), Some((0, 8_000)));
        assert_eq!(times(place(&plan, &rec, true)), [5_000], "範囲の外の時刻は除く");
        rec.offset_ms = 2_000;
        assert_eq!(at(place(&plan, &rec, true)), Some((1_500, 10_000)));
        assert_eq!(times(place(&plan, &rec, true)), [5_500], "先頭からの位置にする");
        assert_eq!(at(place(&plan, &rec, false)), Some((0, 8_000)), "手元の時計の範囲は補正しない");
        let outside = ClipPlan { start_ms: 700_000, end_ms: 710_000, ..plan };
        assert_eq!(place(&outside, &rec, true), None);
    }

    #[test]
    fn prune_keeps_newest_and_respects_the_size_cap() {
        let list = [recording(1, 100, 5), recording(2, 300, 5), recording(3, 200, 5)];
        assert_eq!(prune(&list, 2, 100), [1]);
        assert_eq!(prune(&list, 10, 9), [3, 1]);
        assert!(prune(&list[..1], 0, 0).is_empty(), "最新の 1 本は残す");
    }

    #[test]
    fn kill_events_carry_weapon_and_victim_agent() {
        let mut catalog = GameCatalog::default();
        catalog.agents.insert("a-jett".into(), "Jett".into());
        catalog.weapons.insert("w-vandal".into(), "Vandal".into());
        let mut data = details(vec![json!({"winningTeam": "Red", "playerStats": [{"subject": "me", "kills": [
            {"killer": "me", "victim": "e1", "timeSinceGameStartMillis": 60_000, "timeSinceRoundStartMillis": 20_000,
             "finishingDamage": {"damageType": "Weapon", "damageItem": "W-VANDAL"}}
        ]}]})]);
        data["players"][2]["characterId"] = json!("A-JETT");
        let event = &plan(&data, "me", &catalog)[0].events[0];
        assert_eq!((event.weapon.as_deref(), event.victim.as_deref()), (Some("Vandal"), Some("Jett")));
    }
}
