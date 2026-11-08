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

/// Riot が発行する参加リンクの先頭。これ以外の URL は外へ出さない。
const SMART_URL_ROOT: &str = "https://gg.riotgames.com/";

/// Riot Client の activity-gateway が発行する、期限付きの参加リンク。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinLink {
    pub url: String,
    pub expires_ms: i64,
}

impl JoinLink {
    pub fn valid_at(&self, now_ms: i64) -> bool {
        self.expires_ms > now_ms
    }
}

/// 自分が入っている VALORANT のパーティ。activity ID はパーティ ID とは別の値。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyActivity {
    pub activity_id: String,
    pub link: Option<JoinLink>,
}

/// URL のパスに入れるので、英数字と `-`、`_` だけを通す。
fn safe_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `JoinCodeV1` を読む。`expiresAt` はエポックミリ秒で、数値と文字列の両方がある。
pub fn join_link(value: &Value) -> Option<JoinLink> {
    if value.get("isActive").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let url = value.get("smartUrl")?.as_str()?;
    let tail = url.strip_prefix(SMART_URL_ROOT)?;
    let allowed = |c: char| c.is_ascii_alphanumeric() || "-_?=&/".contains(c);
    if tail.is_empty() || !tail.chars().all(allowed) {
        return None;
    }
    let expires = value.get("expiresAt")?;
    let expires_ms = expires.as_i64().or_else(|| expires.as_str()?.parse().ok())?;
    Some(JoinLink { url: url.to_owned(), expires_ms })
}

/// `/activity-gateway/v1/activities` から、自分が入っている VALORANT のパーティを探す。
pub fn party_activity(value: &Value) -> Option<PartyActivity> {
    value.get("activities")?.as_object()?.values().find_map(|activity| {
        let text = |key| activity.get(key).and_then(Value::as_str);
        let member = activity.get("isMember").and_then(Value::as_bool) == Some(true);
        if !member || text("product") != Some("valorant") || text("type") != Some("party") {
            return None;
        }
        let activity_id = text("activityId").filter(|id| safe_id(id))?.to_owned();
        Some(PartyActivity { activity_id, link: activity.get("joinCode").and_then(join_link) })
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

    fn activities() -> Value {
        json!({"activities": {
            "other": {"activityId": "lolOne", "product": "league_of_legends", "type": "party", "isMember": true},
            "kWWRLUs9MUla": {
                "activityId": "kWWRLUs9MUla", "id": "6e618738-7e9a-47d6-a3df-4c48c2dc0e21",
                "isMember": true, "product": "valorant", "type": "party",
                "joinCode": {"expiresAt": 1794206154000_i64, "isActive": true, "joinCode": "83PS-H8IB-Yilg",
                    "smartUrl": "https://gg.riotgames.com/VAL?joinCode=83PS-H8IB-Yilg"}
            }
        }})
    }

    #[test]
    fn finds_the_valorant_party_and_its_link() {
        let activity = party_activity(&activities()).expect("activity");
        assert_eq!(activity.activity_id, "kWWRLUs9MUla");
        let link = activity.link.expect("link");
        assert_eq!(link.url, "https://gg.riotgames.com/VAL?joinCode=83PS-H8IB-Yilg");
        assert!(link.valid_at(1794206153999));
        assert!(!link.valid_at(1794206154000));
    }

    #[test]
    fn rejects_links_outside_riot_and_inactive_codes() {
        let code = |url: &str, active| json!({"isActive": active, "smartUrl": url, "expiresAt": "1794206154000"});
        assert!(join_link(&code("https://gg.riotgames.com/VAL?joinCode=AB-CD", true)).is_some());
        assert_eq!(join_link(&code("https://gg.riotgames.com/VAL?joinCode=AB-CD", true)).unwrap().expires_ms, 1794206154000);
        assert!(join_link(&code("https://gg.riotgames.com/VAL?joinCode=AB-CD", false)).is_none());
        assert!(join_link(&code("https://evil.example/VAL?joinCode=AB", true)).is_none());
        assert!(join_link(&code("https://gg.riotgames.com.evil.example/x", true)).is_none());
        assert!(join_link(&code("https://gg.riotgames.com/VAL?x=<script>", true)).is_none());
    }

    #[test]
    fn skips_activities_with_unsafe_ids_or_without_membership() {
        let value = json!({"activities": {"a": {"activityId": "../x", "product": "valorant", "type": "party", "isMember": true},
            "b": {"activityId": "abc", "product": "valorant", "type": "party", "isMember": false}}});
        assert_eq!(party_activity(&value), None);
    }
}
