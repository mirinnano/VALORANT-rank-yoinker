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
    /// 確実にヘッドショットで倒したか。古い記録には無いので、無ければ偽。
    #[serde(default)]
    pub headshot: bool,
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

/// そのラウンドで、倒した人が相手に当てた弾がすべて頭だったか。キルごとの部位は
/// 試合詳細に無いので、頭だけに当てていたときだけヘッドショットとみなす。
fn headshot_kill(round: &Value, kill: &Value) -> bool {
    let (Some(killer), Some(victim)) = (text(kill, "killer"), text(kill, "victim")) else { return false };
    let count = |damage: &Value, key| damage.get(key).and_then(Value::as_i64).unwrap_or(0);
    let mut head = 0;
    let mut other = 0;
    for player in list(round, "playerStats").iter().filter(|p| text(p, "subject") == Some(killer)) {
        for damage in list(player, "damage").iter().filter(|d| text(d, "receiver") == Some(victim)) {
            head += count(damage, "headshots");
            other += count(damage, "bodyshots") + count(damage, "legshots");
        }
    }
    head > 0 && other == 0
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
    let kill_event = |round: &Value, kill: &Value| -> Option<ClipEvent> {
        Some(ClipEvent {
            at_ms: game_start + game_time(kill)?,
            kind: EventKind::Kill,
            weapon: crate::review::weapon_name(kill, catalog),
            victim: text(kill, "victim").and_then(|v| agents.get(v)).cloned(),
            headshot: headshot_kill(round, kill),
        })
    };
    let defuse_event = |at_ms| ClipEvent { at_ms, kind: EventKind::Defuse, weapon: None, victim: None, headshot: false };

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
                    events: streak.iter().filter_map(|kill| kill_event(round, kill)).collect(),
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
                .filter_map(|kill| kill_event(round, kill))
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

/// 自動で時刻を合わせるのに要る、終了時刻を比べられたラウンドの数。
const MIN_SYNC_ROUNDS: usize = 3;

/// 試合詳細から、終了時刻がはっきりしているラウンドの終わりを Riot 側の時刻で返す。
/// 全滅で終わったラウンドは最後のキル、解除で終わったラウンドは解除の瞬間。
/// 番号は 1 始まり。
pub fn round_ends(details: &Value) -> Vec<(u32, i64)> {
    let Some(game_start) = details.pointer("/matchInfo/gameStartMillis").and_then(Value::as_i64) else {
        return Vec::new();
    };
    list(details, "roundResults")
        .iter()
        .enumerate()
        .filter_map(|(index, round)| {
            let kills: Vec<&Value> = list(round, "playerStats").iter().flat_map(|p| list(p, "kills")).collect();
            let end = match text(round, "roundResult") {
                Some("Eliminated") => kills.iter().filter_map(|kill| game_time(kill)).max()?,
                Some("Bomb defused") => {
                    let start = kills.iter().find_map(|kill| Some(game_time(kill)? - round_time(kill)?))?;
                    start + round.get("defuseRoundTime")?.as_i64()?
                }
                _ => return None,
            };
            Some((index as u32 + 1, game_start + end))
        })
        .collect()
}

/// 試合中に手元で見たラウンドの終わり `marks` と、試合詳細の終わりを比べて、Riot 側の
/// 時刻へ足す値を求める。外れ値に引きずられないよう中央値を使う。比べられるラウンドが
/// 少なければ `None`。手元の終わりは、スコアの更新を受け取った時刻。
pub fn estimate_offset(details: &Value, marks: &[(u32, i64)]) -> Option<i64> {
    let local: HashMap<u32, i64> = marks.iter().copied().collect();
    let mut deltas: Vec<i64> =
        round_ends(details).into_iter().filter_map(|(round, riot)| Some(local.get(&round)? - riot)).collect();
    if deltas.len() < MIN_SYNC_ROUNDS {
        return None;
    }
    deltas.sort_unstable();
    Some(deltas[deltas.len() / 2])
}

/// 試合の中の出来事の種類。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Kill,
    Plant,
    Defuse,
}

/// 出来事をした人と自分の関係。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionSide {
    Own,
    Ally,
    Enemy,
}

/// 試合の全員の出来事。名前は持たず、エージェントだけを持つ。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchAction {
    /// Riot 側のエポックミリ秒。録画に合わせると、録画の先頭からのミリ秒になる。
    pub at_ms: i64,
    pub round: u32,
    pub kind: ActionKind,
    pub actor: Option<String>,
    pub actor_side: Option<ActionSide>,
    pub target: Option<String>,
    pub target_side: Option<ActionSide>,
    pub weapon: Option<String>,
    /// 確実にヘッドショットで倒したか。キル以外は偽。
    pub headshot: bool,
}

/// 試合詳細から、全員のキル、設置、解除を時刻順に並べる。
pub fn match_actions(details: &Value, own: &str, catalog: &GameCatalog) -> Vec<MatchAction> {
    let Some(game_start) = details.pointer("/matchInfo/gameStartMillis").and_then(Value::as_i64) else {
        return Vec::new();
    };
    let players = list(details, "players");
    let team = players.iter().find(|p| text(p, "subject") == Some(own)).and_then(|p| text(p, "teamId"));
    let team_of: HashMap<&str, &str> =
        players.iter().filter_map(|p| Some((text(p, "subject")?, text(p, "teamId")?))).collect();
    let agents: HashMap<&str, String> = players
        .iter()
        .filter_map(|p| Some((text(p, "subject")?, catalog.agents.get(&text(p, "characterId")?.to_lowercase())?.clone())))
        .collect();
    let side = |puuid: Option<&str>| -> Option<ActionSide> {
        let puuid = puuid?;
        Some(if puuid == own {
            ActionSide::Own
        } else if team.is_some() && team_of.get(puuid).copied() == team {
            ActionSide::Ally
        } else {
            ActionSide::Enemy
        })
    };
    let agent = |puuid: Option<&str>| puuid.and_then(|id| agents.get(id)).cloned();

    let mut actions = Vec::new();
    for (index, round) in list(details, "roundResults").iter().enumerate() {
        let number = index as u32 + 1;
        let kills: Vec<&Value> = list(round, "playerStats").iter().flat_map(|p| list(p, "kills")).collect();
        let round_start = kills.iter().find_map(|kill| Some(game_time(kill)? - round_time(kill)?));
        for kill in &kills {
            let Some(at) = game_time(kill) else { continue };
            let (killer, victim) = (text(kill, "killer"), text(kill, "victim"));
            actions.push(MatchAction {
                at_ms: game_start + at,
                round: number,
                kind: ActionKind::Kill,
                actor: agent(killer),
                actor_side: side(killer),
                target: agent(victim),
                target_side: side(victim),
                weapon: crate::review::weapon_name(kill, catalog),
                headshot: headshot_kill(round, kill),
            });
        }
        let Some(start) = round_start else { continue };
        for (kind, time_key, who_key) in
            [(ActionKind::Plant, "plantRoundTime", "bombPlanter"), (ActionKind::Defuse, "defuseRoundTime", "bombDefuser")]
        {
            let Some(at) = round.get(time_key).and_then(Value::as_i64).filter(|at| *at > 0) else { continue };
            let who = text(round, who_key).filter(|id| !id.is_empty());
            actions.push(MatchAction {
                at_ms: game_start + start + at,
                round: number,
                kind,
                actor: agent(who),
                actor_side: side(who),
                target: None,
                target_side: None,
                weapon: None,
                headshot: false,
            });
        }
    }
    actions.sort_by_key(|action| action.at_ms);
    actions
}

/// 録画の中の位置に直す。録画の外の出来事は除く。
pub fn place_actions(actions: Vec<MatchAction>, recording: &Recording) -> Vec<MatchAction> {
    actions
        .into_iter()
        .map(|action| MatchAction { at_ms: action.at_ms + recording.offset_ms - recording.started_ms, ..action })
        .filter(|action| (0..=recording.length_ms).contains(&action.at_ms))
        .collect()
}

/// 画面で選んだ範囲を、ホットキーと同じ手動のクリップにする。範囲は録画の先頭からのミリ秒。
/// `actions` は [`place_actions`] で録画に合わせたもの。範囲の中の自分のキルと解除を
/// 見せ場にする。`title` が空なら、キルの数から付ける。
pub fn chosen(
    recording: &Recording,
    round: Option<u32>,
    start_ms: i64,
    end_ms: i64,
    title: &str,
    actions: &[MatchAction],
) -> Option<(ClipPlan, Placement)> {
    let start = start_ms.max(0);
    let end = end_ms.min(recording.length_ms);
    if end - start < 1_000 {
        return None;
    }
    let events: Vec<ClipEvent> = actions
        .iter()
        .filter(|action| action.actor_side == Some(ActionSide::Own) && (start..=end).contains(&action.at_ms))
        .filter_map(|action| {
            let kind = match action.kind {
                ActionKind::Kill => EventKind::Kill,
                ActionKind::Defuse => EventKind::Defuse,
                ActionKind::Plant => return None,
            };
            Some(ClipEvent {
                at_ms: action.at_ms - start,
                kind,
                weapon: action.weapon.clone(),
                victim: action.target.clone(),
                headshot: action.headshot,
            })
        })
        .collect();
    let kills = events.iter().filter(|event| event.kind == EventKind::Kill).count() as u32;
    let title = match title.trim() {
        "" if kills > 0 => streak_title(kills),
        "" => "選んだ場面".to_owned(),
        text => text.to_owned(),
    };
    let plan = ClipPlan {
        kind: ClipKind::Manual,
        round,
        start_ms: recording.started_ms + start,
        end_ms: recording.started_ms + end,
        kills,
        title,
        events: events.clone(),
    };
    Some((plan, Placement { start_ms: start, end_ms: end, events }))
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

/// [`place`] に加えて、キルの数と題を録画に映っている分だけで数え直す。録画の始まる
/// 前のキルまで数えると、映っていない 2K が出てしまう。映るキルが無くなった
/// キルのクリップは作らない。
pub fn fit(mut plan: ClipPlan, recording: &Recording, riot_clock: bool) -> Option<(ClipPlan, Placement)> {
    let at = place(&plan, recording, riot_clock)?;
    if plan.kind == ClipKind::Kills {
        let kills = at.events.iter().filter(|event| event.kind == EventKind::Kill).count() as u32;
        if kills == 0 {
            return None;
        }
        plan.kills = kills;
        plan.title = streak_title(kills);
    }
    Some((plan, at))
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
            events: [6_000, 50_000].map(|at_ms| ClipEvent { at_ms, kind: EventKind::Kill, weapon: None, victim: None, headshot: false }).to_vec(),
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

    fn synced_match() -> Value {
        let kill = |killer: &str, victim: &str, game: i64, round: i64| {
            json!({"killer": killer, "victim": victim, "timeSinceGameStartMillis": game, "timeSinceRoundStartMillis": round})
        };
        json!({
            "matchInfo": {"gameStartMillis": 1_000_000},
            "players": [
                {"subject": "me", "teamId": "Blue", "characterId": "jett"},
                {"subject": "mate", "teamId": "Blue", "characterId": "sova"},
                {"subject": "foe", "teamId": "Red", "characterId": "omen"}
            ],
            "roundResults": [
                {"roundResult": "Eliminated", "playerStats": [{"kills": [kill("me", "foe", 50_000, 20_000), kill("me", "foe", 60_000, 30_000)]}]},
                {"roundResult": "Bomb defused", "defuseRoundTime": 70_000, "bombDefuser": "mate", "plantRoundTime": 40_000, "bombPlanter": "foe",
                 "playerStats": [{"kills": [kill("foe", "mate", 130_000, 10_000)]}]},
                {"roundResult": "Bomb detonated", "playerStats": [{"kills": [kill("mate", "foe", 220_000, 15_000)]}]},
                {"roundResult": "Eliminated", "playerStats": [{"kills": [kill("foe", "me", 300_000, 25_000)]}]}
            ]
        })
    }

    #[test]
    fn round_ends_use_the_last_kill_or_the_defuse() {
        let ends = round_ends(&synced_match());
        // 3 ラウンド目は爆発なので終わりが分からず、使わない。
        assert_eq!(ends, vec![(1, 1_060_000), (2, 1_000_000 + 120_000 + 70_000), (4, 1_300_000)]);
    }

    #[test]
    fn offset_is_the_median_gap_and_needs_three_rounds() {
        let details = synced_match();
        let marks = [(1, 1_060_000 + 4_000), (2, 1_190_000 + 5_000), (4, 1_300_000 + 30_000)];
        assert_eq!(estimate_offset(&details, &marks), Some(5_000), "外れた 1 ラウンドに引きずられない");
        assert_eq!(estimate_offset(&details, &marks[..2]), None);
    }

    #[test]
    fn actions_cover_everyone_with_sides_and_land_inside_the_recording() {
        let mut catalog = GameCatalog::default();
        for (id, name) in [("jett", "Jett"), ("sova", "Sova"), ("omen", "Omen")] {
            catalog.agents.insert(id.into(), name.into());
        }
        let actions = match_actions(&synced_match(), "me", &catalog);
        assert_eq!(actions.len(), 7, "キル 5 件と設置、解除");
        let plant = actions.iter().find(|a| a.kind == ActionKind::Plant).unwrap();
        assert_eq!((plant.at_ms, plant.actor.as_deref(), plant.actor_side), (1_160_000, Some("Omen"), Some(ActionSide::Enemy)));
        let first = &actions[0];
        assert_eq!((first.actor_side, first.target_side), (Some(ActionSide::Own), Some(ActionSide::Enemy)));

        let rec = Recording {
            id: 1, match_id: None, path: String::new(), started_ms: 1_055_000, length_ms: 200_000, bytes: 0, offset_ms: 0,
            map: None, mode: None, width: None, height: None,
        };
        let placed = place_actions(actions, &rec);
        assert_eq!(placed[0].at_ms, 5_000, "録画の前の出来事は除き、先頭からの位置にする");
        assert!(placed.iter().all(|a| a.at_ms <= 200_000));
    }

    #[test]
    fn chosen_range_is_a_manual_clip_clamped_to_the_recording() {
        let rec = Recording {
            id: 1, match_id: None, path: String::new(), started_ms: 10_000, length_ms: 60_000, bytes: 0, offset_ms: 0,
            map: None, mode: None, width: None, height: None,
        };
        let action = |at_ms, side| MatchAction {
            at_ms, round: 3, kind: ActionKind::Kill, actor: Some("Jett".into()), actor_side: Some(side),
            target: Some("Omen".into()), target_side: Some(ActionSide::Enemy), weapon: Some("Vandal".into()),
            headshot: at_ms == 14_000,
        };
        let actions = [action(10_000, ActionSide::Own), action(12_000, ActionSide::Ally), action(14_000, ActionSide::Own), action(90_000, ActionSide::Own)];
        let (plan, at) = chosen(&rec, Some(3), -2_000, 70_000, "", &actions).unwrap();
        assert_eq!((plan.kind, at.start_ms, at.end_ms, plan.round), (ClipKind::Manual, 0, 60_000, Some(3)));
        assert_eq!((plan.kills, plan.title.as_str()), (2, "2K"), "範囲の中の自分のキルだけを数える");
        assert_eq!(at.events.iter().map(|e| e.at_ms).collect::<Vec<_>>(), vec![10_000, 14_000]);
        assert_eq!(at.events.iter().map(|e| e.headshot).collect::<Vec<_>>(), vec![false, true]);
        let (named, _) = chosen(&rec, None, 0, 5_000, "見て", &actions).unwrap();
        assert_eq!((named.title.as_str(), named.kills), ("見て", 0));
        assert!(chosen(&rec, None, 5_000, 5_500, "x", &actions).is_none());
    }

    #[test]
    fn fit_counts_only_kills_inside_the_recording() {
        let event = |at_ms| ClipEvent { at_ms, kind: EventKind::Kill, weapon: None, victim: None, headshot: false };
        let plan = ClipPlan {
            kind: ClipKind::Kills, round: Some(1), start_ms: 1_000, end_ms: 20_000, kills: 2, title: "2K".into(),
            events: vec![event(5_000), event(15_000)],
        };
        let rec = Recording {
            id: 1, match_id: None, path: String::new(), started_ms: 10_000, length_ms: 60_000, bytes: 0, offset_ms: 0,
            map: None, mode: None, width: None, height: None,
        };
        let (fitted, at) = fit(plan.clone(), &rec, true).unwrap();
        assert_eq!((fitted.kills, fitted.title.as_str(), at.events.len()), (1, "キル", 1));
        let late = Recording { started_ms: 16_000, ..rec };
        assert!(fit(plan, &late, true).is_none(), "映るキルが無ければ作らない");
    }

    #[test]
    fn headshot_only_when_every_hit_on_the_victim_was_to_the_head() {
        let round = |head: i64, body: i64| {
            json!({"playerStats": [
                {"subject": "me", "damage": [
                    {"receiver": "foe", "headshots": head, "bodyshots": body, "legshots": 0},
                    {"receiver": "other", "headshots": 0, "bodyshots": 3, "legshots": 1}
                ]},
                {"subject": "foe", "damage": [{"receiver": "me", "headshots": 0, "bodyshots": 2, "legshots": 0}]}
            ]})
        };
        let kill = json!({"killer": "me", "victim": "foe"});
        assert!(headshot_kill(&round(1, 0), &kill));
        assert!(!headshot_kill(&round(1, 2), &kill), "胴にも当てていれば、どこで倒したか分からない");
        assert!(!headshot_kill(&round(0, 0), &kill), "当てた記録が無ければヘッドショットとしない");
    }
}
