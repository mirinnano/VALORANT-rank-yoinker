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
    pub acs: Option<u32>,
    pub adr: Option<u32>,
    pub kast: Option<u32>,
    pub first_kill_percent: Option<u32>,
    pub first_death_percent: Option<u32>,
    pub damage_delta: Option<i32>,
    pub econ: Option<u32>,
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
    /// エージェント選択の残り時間。
    pub phase_remaining_ms: Option<u64>,
    /// エージェント選択で使う、このマップの観測データ。
    pub pick_board: Option<PickBoard>,
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
            phase_remaining_ms: None,
            pick_board: None,
            finished_match: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PickStat {
    pub agent: String,
    pub picks: u32,
    pub pick_percent: u32,
    pub win_percent: Option<u32>,
    /// このマップでの自分の成績。
    pub own_games: u32,
    pub own_wins: u32,
    pub own_acs: Option<u32>,
}

/// 味方のエージェントと同じチームだった試合の成績。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PickPair {
    pub agent: String,
    pub ally: String,
    pub games: u32,
    pub wins: u32,
}

/// 自分のロビーで観測した、マップごとのピックの傾向。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PickBoard {
    pub map: String,
    pub matches: u32,
    /// ランク帯で足りず、全ランクの観測を使った。
    pub all_ranks: bool,
    pub low_tier: u32,
    pub high_tier: u32,
    pub agents: Vec<PickStat>,
    pub pairs: Vec<PickPair>,
}

/// UI へ流すイベント。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppEvent {
    Connecting { reason: Option<String> },
    Snapshot(MatchSnapshot),
}
