//! プレゼンスの解析。
//!
//! Riot の private プレゼンスには、値が直下にある形式と `*PresenceData` の
//! 下にある形式が混在する。[`lookup`] で両方を同じように読む。

use crate::model::{GameState, LobbyPhase};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Presence {
    pub puuid: String,
    #[serde(default)]
    pub private: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default, rename = "championId")]
    pub champion_id: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivatePresence {
    pub state: Option<GameState>,
    pub queue_id: Option<String>,
    pub custom_game: bool,
    pub party_id: Option<String>,
    pub party_size: u32,
    pub party_state: Option<String>,
    pub max_party_size: u32,
    pub party_open: bool,
    pub idle: bool,
    /// 進行中の試合のスコア。自分のチーム、相手の順。
    pub score: Option<(u32, u32)>,
}

const PRESENCE_URI: &str = "/chat/v4/presences";
const NESTED: [&str; 3] = ["matchPresenceData", "partyPresenceData", "playerPresenceData"];

fn lookup<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    root.get(key)
        .or_else(|| NESTED.iter().find_map(|nest| root.get(nest)?.get(key)))
}

fn lookup_str<'a>(root: &'a Value, key: &str) -> Option<&'a str> {
    lookup(root, key)?.as_str()
}

fn match_score(value: &Value) -> Option<(u32, u32)> {
    let number = |key| lookup(value, key).and_then(Value::as_u64).map(|n| n as u32);
    Some((number("partyOwnerMatchScoreAllyTeam")?, number("partyOwnerMatchScoreEnemyTeam")?))
}

/// 平坦な形式と入れ子の形式のどちらかに `key` があるか。
pub fn has_key(root: &Value, key: &str) -> bool {
    lookup(root, key).is_some()
}

impl Presence {
    /// private の JSON をそのまま返す。診断で項目を調べるために使う。
    pub fn private_value(&self) -> Option<Value> {
        let raw = self.private.as_deref().filter(|raw| !raw.is_empty())?;
        serde_json::from_slice(&STANDARD.decode(raw).ok()?).ok()
    }

    fn is_league(&self) -> bool {
        self.champion_id.is_some() || self.product.as_deref() == Some("league_of_legends")
    }

    /// private を解析する。VALORANT 以外の製品や壊れた値は `None`。
    pub fn decode(&self) -> Option<PrivatePresence> {
        if self.is_league() {
            return None;
        }
        let value = self.private_value()?;
        Some(PrivatePresence {
            state: lookup_str(&value, "sessionLoopState").and_then(GameState::parse),
            queue_id: lookup_str(&value, "queueId").map(str::to_owned),
            custom_game: lookup_str(&value, "provisioningFlow") == Some("CustomGame")
                || lookup_str(&value, "partyState") == Some("CUSTOM_GAME_SETUP"),
            party_id: lookup_str(&value, "partyId")
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
            party_size: lookup(&value, "partySize").and_then(Value::as_u64).unwrap_or(0) as u32,
            party_state: lookup_str(&value, "partyState").map(str::to_owned),
            max_party_size: lookup(&value, "maxPartySize").and_then(Value::as_u64).unwrap_or(0) as u32,
            party_open: lookup_str(&value, "partyAccessibility") == Some("OPEN"),
            idle: lookup(&value, "isIdle").and_then(Value::as_bool).unwrap_or(false),
            score: match_score(&value),
        })
    }
}

impl PrivatePresence {
    /// 未知の `partyState` は、通常のロビーとして扱う。
    pub fn lobby_phase(&self) -> LobbyPhase {
        match self.party_state.as_deref() {
            Some("MATCHMAKING") => LobbyPhase::Queueing,
            Some("MATCHMADE_GAME_STARTING") => LobbyPhase::MatchFound,
            _ if self.custom_game => LobbyPhase::Custom,
            _ => LobbyPhase::Ready,
        }
    }
}

/// Riot Client の WebSocket で受け取るイベント。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    OwnPresence(PrivatePresence),
    PartyChanged,
    PregameChanged,
}

const MESSAGE_URI: &str = "/riot-messaging-service/v1/message/";

/// WebSocket のメッセージを解釈する。他人のプレゼンスや無関係な通知は `None`。
pub fn parse_live_event(message: &str, puuid: &str) -> Option<LiveEvent> {
    let value: Value = serde_json::from_str(message).ok()?;
    let payload = value.as_array()?.get(2)?;
    let uri = payload.get("uri")?.as_str()?;
    if let Some(topic) = uri.strip_prefix(MESSAGE_URI) {
        return if topic.starts_with("ares-parties/") {
            Some(LiveEvent::PartyChanged)
        } else if topic.starts_with("ares-pregame/") {
            Some(LiveEvent::PregameChanged)
        } else {
            None
        };
    }
    if uri != PRESENCE_URI {
        return None;
    }
    // 形式の違う項目が混ざっても、自分の分は読めるように 1 件ずつ解釈する。
    payload
        .get("data")?
        .get("presences")?
        .as_array()?
        .iter()
        .filter(|entry| entry.get("puuid").and_then(Value::as_str) == Some(puuid))
        .find_map(|entry| Presence::deserialize(entry).ok()?.decode())
        .map(LiveEvent::OwnPresence)
}

/// 自分のプレゼンスを探す。
pub fn find_own(presences: &[Presence], puuid: &str) -> Option<PrivatePresence> {
    presences.iter().find(|p| p.puuid == puuid).and_then(Presence::decode)
}

/// 自分と同じパーティのプレイヤー。
pub fn party_members(presences: &[Presence], own: &str) -> Vec<String> {
    let Some(own_party) = find_own(presences, own).and_then(|p| p.party_id) else {
        return vec![own.to_owned()];
    };
    presences
        .iter()
        .filter(|p| {
            p.puuid == own
                || p.decode().and_then(|d| d.party_id).as_deref() == Some(own_party.as_str())
        })
        .map(|p| p.puuid.clone())
        .collect()
}

/// `players` の並び順で、同じ試合に2人以上いるパーティへ 1 から番号を振る。
///
/// プレゼンスに載るのはフレンドなどに限られる。`known` は、パーティ API など
/// プレゼンス以外で分かった組で、プレゼンスより優先する。
pub fn party_numbers(presences: &[Presence], players: &[String], known: &[Vec<String>]) -> HashMap<String, u32> {
    let mut parties: HashMap<&str, String> = presences
        .iter()
        .filter_map(|p| {
            let decoded = p.decode()?;
            (decoded.party_size > 1).then_some((p.puuid.as_str(), decoded.party_id?))
        })
        .collect();
    for (index, group) in known.iter().enumerate() {
        for puuid in group {
            parties.insert(puuid.as_str(), format!("known-{index}"));
        }
    }
    let mut counts: HashMap<&str, u32> = HashMap::new();
    for puuid in players {
        if let Some(party) = parties.get(puuid.as_str()) {
            *counts.entry(party).or_default() += 1;
        }
    }

    let mut numbers: HashMap<String, u32> = HashMap::new();
    let mut result = HashMap::new();
    for puuid in players {
        let Some(party) = parties.get(puuid.as_str()) else { continue };
        if counts[party.as_str()] < 2 {
            continue;
        }
        let next = numbers.len() as u32 + 1;
        let number = *numbers.entry(party.clone()).or_insert(next);
        result.insert(puuid.clone(), number);
    }
    result
}

/// private プレゼンスを Riot と同じ base64 形式に変換する。再生用。
pub fn encode_private(value: &Value) -> String {
    STANDARD.encode(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn presence(puuid: &str, private: Value) -> Presence {
        Presence {
            puuid: puuid.into(),
            private: Some(encode_private(&private)),
            product: Some("valorant".into()),
            champion_id: None,
        }
    }

    #[test]
    fn decodes_flat_shape() {
        let p = presence("a", json!({
            "sessionLoopState": "PREGAME", "queueId": "competitive",
            "partyId": "x", "partySize": 2, "provisioningFlow": "Matchmaking"
        }));
        let d = p.decode().unwrap();
        assert_eq!(d.state, Some(GameState::Pregame));
        assert_eq!(d.queue_id.as_deref(), Some("competitive"));
        assert_eq!((d.party_id.as_deref(), d.party_size), (Some("x"), 2));
        assert!(!d.custom_game);
    }

    #[test]
    fn decodes_nested_shape_identically() {
        let flat = presence("a", json!({
            "sessionLoopState": "INGAME", "queueId": "unrated", "partyId": "x", "partySize": 3
        }));
        let nested = presence("a", json!({
            "matchPresenceData": {"sessionLoopState": "INGAME", "queueId": "unrated"},
            "partyPresenceData": {"partyId": "x", "partySize": 3}
        }));
        assert_eq!(flat.decode(), nested.decode());
    }

    #[test]
    fn reads_live_score_from_both_shapes() {
        let flat = presence("a", json!({
            "partyOwnerMatchScoreAllyTeam": 7, "partyOwnerMatchScoreEnemyTeam": 5
        }));
        let nested = presence("a", json!({"matchPresenceData": {
            "partyOwnerMatchScoreAllyTeam": 7, "partyOwnerMatchScoreEnemyTeam": 5
        }}));
        assert_eq!(flat.decode().unwrap().score, Some((7, 5)));
        assert_eq!(nested.decode().unwrap().score, Some((7, 5)));
    }

    #[test]
    fn missing_score_is_none_not_zero() {
        let p = presence("a", json!({"partyOwnerMatchScoreAllyTeam": 7}));
        assert_eq!(p.decode().unwrap().score, None);
    }

    #[test]
    fn reads_lobby_fields_and_phase_from_both_shapes() {
        let flat = presence("a", json!({
            "partyState": "MATCHMAKING", "maxPartySize": 5,
            "partyAccessibility": "OPEN", "isIdle": true
        })).decode().unwrap();
        let nested = presence("a", json!({
            "partyPresenceData": {"partyState": "MATCHMAKING", "maxPartySize": 5, "partyAccessibility": "OPEN"},
            "playerPresenceData": {"isIdle": true}
        })).decode().unwrap();
        assert_eq!(flat, nested);
        assert_eq!(flat.lobby_phase(), LobbyPhase::Queueing);
        assert_eq!((flat.max_party_size, flat.party_open, flat.idle), (5, true, true));
    }

    #[test]
    fn unknown_party_state_is_a_plain_lobby() {
        let p = presence("a", json!({"partyState": "SOMETHING_NEW"})).decode().unwrap();
        assert_eq!(p.lobby_phase(), LobbyPhase::Ready);
        let custom = presence("a", json!({"partyState": "CUSTOM_GAME_SETUP"})).decode().unwrap();
        assert_eq!(custom.lobby_phase(), LobbyPhase::Custom);
    }

    #[test]
    fn detects_custom_game_from_party_state() {
        let p = presence("a", json!({"partyPresenceData": {"partyState": "CUSTOM_GAME_SETUP"}}));
        assert!(p.decode().unwrap().custom_game);
    }

    #[test]
    fn ignores_league_and_broken_values() {
        let mut league = presence("a", json!({"sessionLoopState": "MENUS"}));
        league.product = Some("league_of_legends".into());
        assert!(league.decode().is_none());

        let mut broken = presence("a", json!({}));
        broken.private = Some("***".into());
        assert!(broken.decode().is_none());

        broken.private = Some(String::new());
        assert!(broken.decode().is_none());
    }

    fn event(presence: &Presence) -> String {
        let data = json!({"uri": "/chat/v4/presences", "data": {"presences": [{
            "puuid": presence.puuid, "private": presence.private, "product": "valorant"
        }]}});
        json!([8, "OnJsonApiEvent_chat_v4_presences", data]).to_string()
    }

    #[test]
    fn reads_own_state_from_a_websocket_message() {
        let own = presence("me", json!({"sessionLoopState": "PREGAME", "partyState": "DEFAULT"}));
        let Some(LiveEvent::OwnPresence(found)) = parse_live_event(&event(&own), "me") else {
            panic!("own presence expected");
        };
        assert_eq!(found.state, Some(GameState::Pregame));
    }

    #[test]
    fn ignores_other_players_and_other_messages() {
        let friend = presence("friend", json!({"sessionLoopState": "INGAME"}));
        assert!(parse_live_event(&event(&friend), "me").is_none());
        assert!(parse_live_event("[5, \"subscribe\"]", "me").is_none());
        assert!(parse_live_event("not json", "me").is_none());
    }

    #[test]
    fn a_malformed_entry_does_not_hide_own_presence() {
        let own = presence("me", json!({"sessionLoopState": "INGAME"}));
        let data = json!({"uri": "/chat/v4/presences", "data": {"presences": [
            {"unexpected": true},
            {"puuid": "me", "private": own.private, "product": "valorant"}
        ]}});
        let message = json!([8, "OnJsonApiEvent_chat_v4_presences", data]).to_string();
        assert!(matches!(parse_live_event(&message, "me"), Some(LiveEvent::OwnPresence(_))));
    }

    #[test]
    fn messaging_events_are_classified_by_service() {
        let message = |uri: &str| json!([8, "OnJsonApiEvent_riot-messaging-service_v1_message", {"uri": uri}]).to_string();
        let party = message("/riot-messaging-service/v1/message/ares-parties/parties/v1/parties/x");
        let pregame = message("/riot-messaging-service/v1/message/ares-pregame/pregame/v1/matches/x");
        let other = message("/riot-messaging-service/v1/message/ares-core-game/core-game/v1/matches/x");
        assert_eq!(parse_live_event(&party, "me"), Some(LiveEvent::PartyChanged));
        assert_eq!(parse_live_event(&pregame, "me"), Some(LiveEvent::PregameChanged));
        assert_eq!(parse_live_event(&other, "me"), None);
    }

    #[test]
    fn numbers_only_parties_with_two_players_in_the_match() {
        let presences = vec![
            presence("a", json!({"partyId": "p1", "partySize": 2})),
            presence("b", json!({"partyId": "p1", "partySize": 2})),
            // 同じパーティの相方が試合にいないので番号は付かない。
            presence("c", json!({"partyId": "p2", "partySize": 2})),
            presence("d", json!({"partyId": "p3", "partySize": 3})),
            presence("e", json!({"partyId": "p3", "partySize": 3})),
        ];
        let players: Vec<String> = ["d", "a", "c", "e", "b"].map(String::from).to_vec();
        let n = party_numbers(&presences, &players, &[]);
        assert_eq!((n["d"], n["e"]), (1, 1));
        assert_eq!((n["a"], n["b"]), (2, 2));
        assert!(!n.contains_key("c"));
    }

    #[test]
    fn party_members_include_self_without_presence() {
        assert_eq!(party_members(&[], "me"), vec!["me".to_string()]);
    }

    #[test]
    fn party_api_members_are_grouped_without_presence() {
        let presences = vec![presence("friend", json!({"partyId": "p9", "partySize": 2}))];
        let players: Vec<String> = ["me", "stranger", "friend", "other"].iter().map(|s| s.to_string()).collect();
        let known = vec![vec!["me".to_owned(), "stranger".to_owned()]];
        let numbers = party_numbers(&presences, &players, &known);
        assert_eq!(numbers.get("me"), Some(&1));
        assert_eq!(numbers.get("stranger"), Some(&1));
        assert_eq!(numbers.get("friend"), None, "1人だけの組には番号を振らない");
    }
}
