//! 試合の生データを、UI に見せてよいスナップショットへ変換する。

use crate::error::{Error, Result};
use crate::model::{GameState, MatchSnapshot, PlayerView, Relation, RankView};
use crate::rank::{self, Peak, SeasonRank};
use crate::source::GameCatalog;
use crate::stats::RecentStats;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Identity {
    pub account_level: u32,
    pub incognito: bool,
    pub hide_account_level: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RawPlayer {
    pub subject: String,
    #[serde(default, rename = "TeamID")]
    pub team_id: Option<String>,
    #[serde(default, rename = "CharacterID")]
    pub character_id: Option<String>,
    #[serde(default)]
    pub character_selection_state: Option<String>,
    #[serde(default)]
    pub player_identity: Identity,
}

#[derive(Debug, Clone)]
pub struct RawMatch {
    pub state: GameState,
    pub match_id: Option<String>,
    pub map_id: Option<String>,
    pub phase_remaining_ms: Option<u64>,
    pub players: Vec<RawPlayer>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn players(value: Option<&Value>, label: &str) -> Result<Vec<RawPlayer>> {
    let list = value.ok_or_else(|| Error::Decode(label.to_owned()))?;
    Ok(serde_json::from_value(list.clone())?)
}

pub fn parse_pregame(value: &Value) -> Result<RawMatch> {
    let team = value.get("AllyTeam");
    let team_id = team.and_then(|t| text(t, "TeamID"));
    let mut list = players(team.and_then(|t| t.get("Players")), "pregame の AllyTeam")?;
    for player in &mut list {
        player.team_id = player.team_id.take().or_else(|| team_id.clone());
    }
    Ok(RawMatch {
        state: GameState::Pregame,
        match_id: text(value, "ID"),
        map_id: text(value, "MapID"),
        phase_remaining_ms: value.get("PhaseTimeRemainingNS").and_then(Value::as_u64).map(|ns| ns / 1_000_000),
        players: list,
    })
}

pub fn parse_coregame(value: &Value) -> Result<RawMatch> {
    let mut list = players(value.get("Players"), "core-game の Players")?;
    // 試合が始まれば全員確定している。
    for player in &mut list {
        player.character_selection_state = None;
    }
    Ok(RawMatch {
        state: GameState::InGame,
        match_id: text(value, "MatchID"),
        map_id: text(value, "MapID"),
        phase_remaining_ms: None,
        players: list,
    })
}

/// 本名を見てよいプレイヤー。自分とパーティ、Streamer Mode でない人に限る。
fn name_is_visible(player: &RawPlayer, own: &str, party: &HashSet<String>) -> bool {
    !player.player_identity.incognito || player.subject == own || party.contains(&player.subject)
}

/// name-service へ問い合わせる puuid。Streamer Mode の本名はここで除外し、取得しない。
pub fn names_to_fetch(players: &[RawPlayer], own: &str, party: &HashSet<String>) -> Vec<String> {
    players
        .iter()
        .filter(|p| name_is_visible(p, own, party))
        .map(|p| p.subject.clone())
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct PlayerExtra {
    pub rank: SeasonRank,
    pub previous: SeasonRank,
    pub peak: Peak,
    pub peak_act: Option<String>,
    pub stats: RecentStats,
}

pub struct BuildContext<'a> {
    pub own: &'a str,
    pub party: &'a HashSet<String>,
    pub names: &'a HashMap<String, String>,
    pub extras: &'a HashMap<String, PlayerExtra>,
    pub encounters: &'a HashMap<String, u32>,
    pub party_numbers: &'a HashMap<String, u32>,
    pub keys: &'a HashMap<String, u32>,
    pub catalog: &'a GameCatalog,
    pub mode: Option<String>,
}

fn rank_view(rank: &SeasonRank) -> RankView {
    rank::view(rank.tier, rank.rr, rank.leaderboard)
}

fn player_view(player: &RawPlayer, my_team: Option<&str>, ctx: &BuildContext) -> PlayerView {
    let is_self = player.subject == ctx.own;
    let in_party = ctx.party.contains(&player.subject);
    let relation = match () {
        _ if is_self => Relation::Own,
        _ if in_party => Relation::Party,
        _ if player.team_id.as_deref() == my_team => Relation::Ally,
        _ => Relation::Enemy,
    };

    let agent = player
        .character_id
        .as_deref()
        .and_then(|id| ctx.catalog.agents.get(&id.to_lowercase()))
        .cloned();

    let name_hidden = !name_is_visible(player, ctx.own, ctx.party);
    let display_name = if name_hidden {
        agent.clone().unwrap_or_else(|| "Player".to_owned())
    } else {
        ctx.names.get(&player.subject).cloned().unwrap_or_else(|| "Unknown".to_owned())
    };

    let level_hidden = player.player_identity.hide_account_level && !is_self && !in_party;
    let extra = ctx.extras.get(&player.subject).cloned().unwrap_or_default();

    PlayerView {
        puuid: player.subject.clone(),
        key: ctx.keys.get(&player.subject).copied().unwrap_or(0),
        display_name,
        name_hidden,
        relation,
        team: player.team_id.clone(),
        agent,
        agent_locked: match player.character_selection_state.as_deref() {
            Some(state) => state == "locked",
            None => true,
        },
        level: (!level_hidden).then_some(player.player_identity.account_level),
        party: ctx.party_numbers.get(&player.subject).copied(),
        rank: rank_view(&extra.rank),
        peak: rank::view(extra.peak.tier, 0, 0),
        peak_act: extra.peak_act.clone(),
        previous_rank: rank_view(&extra.previous),
        win_rate: extra.rank.win_rate,
        games: extra.rank.games,
        kd: extra.stats.kd,
        headshot_percent: extra.stats.headshot_percent,
        earned_rr: extra.stats.earned_rr,
        afk_penalty: extra.stats.afk_penalty,
        last_active_ms: extra.stats.last_active_ms,
        recent_rr: extra.stats.form.clone(),
        acs: extra.stats.acs,
        adr: extra.stats.adr,
        kast: extra.stats.kast,
        first_kill_percent: extra.stats.first_kill_percent,
        first_death_percent: extra.stats.first_death_percent,
        damage_delta: extra.stats.damage_delta,
        econ: extra.stats.econ,
        loading: !ctx.extras.contains_key(&player.subject),
        encounters: ctx.encounters.get(&player.subject).copied().unwrap_or(0),
    }
}

/// 表示順に並べる。味方を先に、各チーム内はランクの高い順。
///
/// 元実装はレベル順に並べていた。非公開のレベルが順序から推測できるため使わない。
pub fn display_order(players: &mut [RawPlayer], my_team: Option<&str>, extras: &HashMap<String, PlayerExtra>) {
    let tier = |p: &RawPlayer| extras.get(&p.subject).map_or(0, |e| e.rank.tier);
    players.sort_by(|a, b| {
        let enemy = |p: &RawPlayer| p.team_id.as_deref() != my_team;
        enemy(a)
            .cmp(&enemy(b))
            .then(tier(b).cmp(&tier(a)))
            .then(a.subject.cmp(&b.subject))
    });
}

pub fn my_team<'a>(players: &'a [RawPlayer], own: &str) -> Option<&'a str> {
    players.iter().find(|p| p.subject == own)?.team_id.as_deref()
}

/// ランクのあるプレイヤーの平均。tier は四捨五入する。
fn average_rank<'a>(players: impl Iterator<Item = &'a PlayerView>) -> Option<RankView> {
    let tiers: Vec<u32> = players.map(|p| p.rank.tier).filter(|tier| *tier > 0).collect();
    let count = tiers.len() as u32;
    (count > 0).then(|| rank::view((tiers.iter().sum::<u32>() + count / 2) / count, 0, 0))
}

pub fn build_snapshot(raw: &RawMatch, ctx: &BuildContext) -> MatchSnapshot {
    let team = my_team(&raw.players, ctx.own);
    let players: Vec<PlayerView> = raw.players.iter().map(|p| player_view(p, team, ctx)).collect();
    let is_enemy = |p: &&PlayerView| p.relation == Relation::Enemy;
    MatchSnapshot {
        ally_average: average_rank(players.iter().filter(|p| !is_enemy(p))),
        enemy_average: average_rank(players.iter().filter(is_enemy)),
        profile: None,
        score: None,
        phase_remaining_ms: raw.phase_remaining_ms,
        pick_board: None,
        finished_match: None,
        state: raw.state,
        match_id: raw.match_id.clone(),
        mode: ctx.mode.clone(),
        map: raw
            .map_id
            .as_ref()
            .and_then(|id| ctx.catalog.maps.get(&id.to_lowercase()))
            .cloned(),
        players,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(subject: &str, team: &str, incognito: bool, hide_level: bool) -> RawPlayer {
        RawPlayer {
            subject: subject.into(),
            team_id: Some(team.into()),
            character_id: Some("JETT-ID".into()),
            character_selection_state: None,
            player_identity: Identity { account_level: 321, incognito, hide_account_level: hide_level },
        }
    }

    struct Fixture {
        party: HashSet<String>,
        names: HashMap<String, String>,
        catalog: GameCatalog,
        empty_extra: HashMap<String, PlayerExtra>,
        empty_count: HashMap<String, u32>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut catalog = GameCatalog::default();
            catalog.agents.insert("jett-id".into(), "Jett".into());
            Self {
                party: HashSet::from(["friend".to_string()]),
                names: HashMap::from([
                    ("me".to_string(), "Me#1".to_string()),
                    ("friend".to_string(), "Friend#2".to_string()),
                    ("public".to_string(), "Public#3".to_string()),
                ]),
                catalog,
                empty_extra: HashMap::new(),
                empty_count: HashMap::new(),
            }
        }

        fn ctx(&self) -> BuildContext<'_> {
            BuildContext {
                own: "me",
                party: &self.party,
                names: &self.names,
                extras: &self.empty_extra,
                encounters: &self.empty_count,
                party_numbers: &self.empty_count,
                keys: &self.empty_count,
                catalog: &self.catalog,
                mode: None,
            }
        }
    }

    fn snapshot(players: Vec<RawPlayer>, fx: &Fixture) -> MatchSnapshot {
        let raw = RawMatch { state: GameState::InGame, match_id: None, map_id: None, phase_remaining_ms: None, players };
        build_snapshot(&raw, &fx.ctx())
    }

    #[test]
    fn hidden_players_are_never_requested_from_name_service() {
        let fx = Fixture::new();
        let list = vec![
            raw("me", "Blue", true, false),
            raw("friend", "Blue", true, false),
            raw("public", "Red", false, false),
            raw("hidden", "Red", true, false),
        ];
        let fetched = names_to_fetch(&list, "me", &fx.party);
        assert_eq!(fetched, ["me", "friend", "public"]);
    }

    #[test]
    fn hidden_player_shows_agent_name_only() {
        let fx = Fixture::new();
        let s = snapshot(vec![raw("me", "Blue", false, false), raw("hidden", "Red", true, false)], &fx);
        let hidden = &s.players[1];
        assert!(hidden.name_hidden);
        assert_eq!(hidden.display_name, "Jett");
    }

    #[test]
    fn hidden_player_without_agent_is_generic() {
        let fx = Fixture::new();
        let mut player = raw("hidden", "Red", true, false);
        player.character_id = None;
        let s = snapshot(vec![raw("me", "Blue", false, false), player], &fx);
        assert_eq!(s.players[1].display_name, "Player");
    }

    #[test]
    fn own_and_party_names_stay_visible_in_streamer_mode() {
        let fx = Fixture::new();
        let s = snapshot(vec![raw("me", "Blue", true, false), raw("friend", "Blue", true, false)], &fx);
        assert_eq!(s.players[0].display_name, "Me#1");
        assert_eq!(s.players[1].display_name, "Friend#2");
        assert_eq!(s.players[0].relation, Relation::Own);
        assert_eq!(s.players[1].relation, Relation::Party);
    }

    #[test]
    fn hidden_levels_are_removed_except_own_and_party() {
        let fx = Fixture::new();
        let s = snapshot(
            vec![
                raw("me", "Blue", false, true),
                raw("friend", "Blue", false, true),
                raw("hidden", "Red", false, true),
                raw("public", "Red", false, false),
            ],
            &fx,
        );
        let levels: Vec<_> = s.players.iter().map(|p| p.level).collect();
        assert_eq!(levels, [Some(321), Some(321), None, Some(321)]);
    }

    // 元実装の漏えいを再発させないための検査。UI へ出る JSON 全体を調べる。
    #[test]
    fn serialized_snapshot_contains_no_hidden_data() {
        let fx = Fixture::new();
        let s = snapshot(
            vec![raw("me", "Blue", false, false), raw("secret-puuid", "Red", true, true)],
            &fx,
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("secret-puuid"));
        let value: Value = serde_json::from_str(&json).unwrap();
        assert!(value["players"][1]["level"].is_null());
        assert!(value["players"][0].get("puuid").is_none());
    }

    #[test]
    fn order_does_not_depend_on_level() {
        let mut low = raw("a", "Red", false, true);
        low.player_identity.account_level = 1;
        let mut high = raw("b", "Red", false, true);
        high.player_identity.account_level = 999;

        let mut first = vec![raw("me", "Blue", false, false), high.clone(), low.clone()];
        let mut second = vec![raw("me", "Blue", false, false), low, high];
        display_order(&mut first, Some("Blue"), &HashMap::new());
        display_order(&mut second, Some("Blue"), &HashMap::new());
        let names = |list: &[RawPlayer]| list.iter().map(|p| p.subject.clone()).collect::<Vec<_>>();
        assert_eq!(names(&first), ["me", "a", "b"]);
        assert_eq!(names(&first), names(&second));
    }

    #[test]
    fn averages_ignore_unranked_players_and_round() {
        let mut fx = Fixture::new();
        let tier = |tier| PlayerExtra {
            rank: SeasonRank { tier, ..Default::default() },
            ..Default::default()
        };
        fx.empty_extra = HashMap::from([
            ("me".to_string(), tier(10)),
            ("public".to_string(), tier(13)),
            ("hidden".to_string(), tier(0)),
        ]);
        let s = snapshot(
            vec![
                raw("me", "Blue", false, false),
                raw("public", "Blue", false, false),
                raw("hidden", "Blue", false, false),
                raw("foe", "Red", false, false),
            ],
            &fx,
        );
        assert_eq!(s.ally_average.unwrap().tier, 12);
        assert!(s.enemy_average.is_none(), "全員ランクなしなら平均は出さない");
    }

    #[test]
    fn coregame_ignores_stale_selection_state() {
        let value = json!({"Players": [{"Subject": "a", "CharacterSelectionState": "selected"}]});
        let m = parse_coregame(&value).unwrap();
        assert!(m.players[0].character_selection_state.is_none());
    }

    #[test]
    fn parses_pregame_and_applies_team() {
        let value = json!({"ID": "m1", "MapID": "/Game/Ascent", "AllyTeam": {"TeamID": "Blue", "Players": [
            {"Subject": "a", "CharacterID": "", "CharacterSelectionState": "selected",
             "PlayerIdentity": {"AccountLevel": 12, "Incognito": true, "HideAccountLevel": false}}
        ]}});
        let m = parse_pregame(&value).unwrap();
        assert_eq!(m.match_id.as_deref(), Some("m1"));
        assert_eq!(m.players[0].team_id.as_deref(), Some("Blue"));
        assert!(m.players[0].player_identity.incognito);
    }

    #[test]
    fn broken_match_is_an_error_not_a_panic() {
        assert!(parse_pregame(&json!({})).is_err());
        assert!(parse_coregame(&json!({"Players": 1})).is_err());
    }
}
