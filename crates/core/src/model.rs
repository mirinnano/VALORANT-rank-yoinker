//! UI へ渡すデータ。ここに入る値は UI に見せてよいものだけにする。

use crate::profile::Profile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameState {
    Menus,
    Pregame,
    InGame,
}

impl GameState {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "MENUS" => Some(Self::Menus),
            "PREGAME" => Some(Self::Pregame),
            "INGAME" => Some(Self::InGame),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Own,
    Party,
    Ally,
    Enemy,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankView {
    pub tier: u32,
    pub name: &'static str,
    pub rr: u32,
    pub leaderboard: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    /// 内部の照合用。UI へは渡さない。
    #[serde(skip)]
    pub puuid: String,
    /// 試合中に同じプレイヤーを追うための連番。puuid とは無関係。
    pub key: u32,
    pub display_name: String,
    pub name_hidden: bool,
    pub relation: Relation,
    pub team: Option<String>,
    pub agent: Option<String>,
    pub agent_locked: bool,
    /// 本人が非公開にしたレベルは `None`。
    pub level: Option<u32>,
    pub party: Option<u32>,
    pub rank: RankView,
    pub peak: RankView,
    pub peak_act: Option<String>,
    pub previous_rank: RankView,
    pub win_rate: Option<u32>,
    pub games: u32,
    pub kd: Option<f64>,
    pub headshot_percent: Option<u32>,
    pub earned_rr: Option<i32>,
    pub afk_penalty: Option<i32>,
    pub last_active_ms: Option<i64>,
    /// 直近の試合の RR 増減。新しい順。
    pub recent_rr: Vec<i32>,
    /// ランクや成績を取得中。
    pub loading: bool,
    /// 過去の遭遇回数。今回の試合は数えない。
    pub encounters: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchSnapshot {
    pub state: GameState,
    pub match_id: Option<String>,
    pub mode: Option<String>,
    pub map: Option<String>,
    pub players: Vec<PlayerView>,
    /// ランクのある味方の平均。
    pub ally_average: Option<RankView>,
    /// ランクのある相手の平均。
    pub enemy_average: Option<RankView>,
    /// 自分のプロフィール。メニューで表示する。
    pub profile: Option<Profile>,
    /// 進行中の試合のスコア。自分のチーム、相手の順。
    pub score: Option<(u32, u32)>,
    /// 直近に終わった試合。振り返りの対象。
    pub finished_match: Option<String>,
}

impl MatchSnapshot {
    pub fn empty(state: GameState, mode: Option<String>) -> Self {
        Self {
            state,
            match_id: None,
            mode,
            map: None,
            players: Vec::new(),
            ally_average: None,
            enemy_average: None,
            profile: None,
            score: None,
            finished_match: None,
        }
    }
}

/// UI へ流すイベント。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppEvent {
    Connecting { reason: Option<String> },
    Snapshot(MatchSnapshot),
}
