//! Riot のパーティ API の解析。
//!
//! プレゼンスにはフレンドしか載らないが、パーティ API には全員が載る。

use serde_json::Value;

const CARD_ROOT: &str = "https://media.valorant-api.com/playercards";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMember {
    pub puuid: String,
    pub card: Option<String>,
    pub title: Option<String>,
    /// 本人が非公開にしたレベルは `None`。
    pub level: Option<u32>,
    pub tier: u32,
    pub owner: bool,
    pub ready: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartyInfo {
    pub members: Vec<RawMember>,
    pub open: bool,
    pub invite_code: Option<String>,
}

/// URL に埋め込むので、UUID の形をした値だけを通す。
fn asset_id(value: &Value, key: &str) -> Option<String> {
    let id = value.get(key)?.as_str()?;
    let valid = id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    (valid && id.chars().any(|c| c != '0' && c != '-')).then(|| id.to_lowercase())
}

/// バナーの URL。`art` は `largeart`（縦長）か `wideart`（横長）。
pub fn card_url(card: &str, art: &str) -> String {
    format!("{CARD_ROOT}/{card}/{art}.png")
}

fn member(value: &Value) -> Option<RawMember> {
    let identity = value.get("PlayerIdentity")?;
    let flag = |root: &Value, key| root.get(key).and_then(Value::as_bool).unwrap_or(false);
    let hidden = flag(identity, "HideAccountLevel");
    Some(RawMember {
        puuid: value.get("Subject")?.as_str()?.to_owned(),
        card: asset_id(identity, "PlayerCardID"),
        title: asset_id(identity, "PlayerTitleID"),
        level: identity
            .get("AccountLevel")
            .and_then(Value::as_u64)
            .filter(|_| !hidden)
            .map(|level| level as u32),
        tier: value.get("CompetitiveTier").and_then(Value::as_u64).unwrap_or(0) as u32,
        owner: flag(value, "IsOwner"),
        ready: flag(value, "IsReady"),
    })
}

pub fn parse(value: &Value) -> Option<PartyInfo> {
    let members: Vec<RawMember> = value.get("Members")?.as_array()?.iter().filter_map(member).collect();
    Some(PartyInfo {
        members,
        open: value.get("Accessibility").and_then(Value::as_str) == Some("OPEN"),
        invite_code: value
            .get("InviteCode")
            .and_then(Value::as_str)
            .filter(|code| !code.is_empty())
            .map(str::to_owned),
    })
}

pub fn current_party_id(player: &Value) -> Option<String> {
    player.get("CurrentPartyID")?.as_str().filter(|id| !id.is_empty()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CARD: &str = "d32e58b1-4191-7315-ad4a-9da58b3f23dd";

    fn raw(hide: bool) -> Value {
        json!({"Accessibility": "OPEN", "InviteCode": "ABC123", "Members": [{
            "Subject": "me", "CompetitiveTier": 15, "IsOwner": true, "IsReady": true,
            "PlayerIdentity": {"PlayerCardID": CARD.to_uppercase(), "PlayerTitleID": "",
                "AccountLevel": 120, "HideAccountLevel": hide}
        }]})
    }

    #[test]
    fn reads_members_and_the_invite_code() {
        let party = parse(&raw(false)).unwrap();
        let own = &party.members[0];
        assert_eq!((own.tier, own.owner, own.ready, own.level), (15, true, true, Some(120)));
        assert_eq!(own.card.as_deref(), Some(CARD));
        assert_eq!(own.title, None);
        assert!(party.open);
        assert_eq!(party.invite_code.as_deref(), Some("ABC123"));
    }

    #[test]
    fn hidden_level_stays_hidden() {
        assert_eq!(parse(&raw(true)).unwrap().members[0].level, None);
    }

    #[test]
    fn an_empty_invite_code_means_no_code() {
        let party = parse(&json!({"Members": [], "InviteCode": ""})).unwrap();
        assert_eq!(party.invite_code, None);
    }

    #[test]
    fn card_ids_that_could_change_the_url_are_rejected() {
        let value = json!({"a": "../../evil/x", "b": "00000000-0000-0000-0000-000000000000"});
        assert_eq!(asset_id(&value, "a"), None);
        assert_eq!(asset_id(&value, "b"), None);
    }
}
