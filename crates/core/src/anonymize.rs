//! 記録を共有できる形へ匿名化する。
//!
//! UUID は公開データのものを除いて仮の値へ置き換える。同じ UUID は同じ仮の値になるので、
//! 記録を再生しても参照関係は保たれる。名前やトークンは値を消す。
//! 出力は [`Anonymizer::verify`] で検査し、元の値が残っていれば保存しない。

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const UUID_LENGTH: usize = 36;
const REDACTED: &str = "redacted";
/// これより短い名前は普通の語と衝突しやすいので、残存検査の対象にしない。
const MIN_SECRET_LENGTH: usize = 5;
/// 長い一続きの文字列はトークンの可能性があるので消す。
const MAX_OPAQUE_LENGTH: usize = 200;

/// 値を消すキー。比較は小文字で行う。
const SENSITIVE_KEYS: [&str; 13] = [
    "gamename", "game_name", "tagline", "game_tag", "displayname", "accesstoken", "token",
    "idtoken", "password", "customgamename", "msg", "body", "pid",
];

fn is_uuid(text: &str) -> bool {
    text.len() == UUID_LENGTH
        && text.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

pub struct Anonymizer {
    public_ids: HashSet<String>,
    pseudonyms: HashMap<String, String>,
    secrets: HashSet<String>,
}

impl Anonymizer {
    /// `public_ids` はエージェントやシーズンなど、公開データの ID。
    pub fn new(public_ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            public_ids: public_ids.into_iter().map(|id| id.to_lowercase()).collect(),
            pseudonyms: HashMap::new(),
            secrets: HashSet::new(),
        }
    }

    fn pseudonym(&mut self, uuid: &str) -> String {
        let id = uuid.to_lowercase();
        if self.public_ids.contains(&id) {
            return uuid.to_owned();
        }
        let next = self.pseudonyms.len() + 1;
        self.pseudonyms
            .entry(id)
            .or_insert_with(|| format!("00000000-0000-4000-8000-{next:012x}"))
            .clone()
    }

    /// 文字列の中の UUID を置き換える。`id@server` のように埋め込まれた場合も対象にする。
    fn text(&mut self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(candidate) = rest.get(..UUID_LENGTH).filter(|part| is_uuid(part)) {
                output.push_str(&self.pseudonym(candidate));
                rest = &rest[UUID_LENGTH..];
            } else {
                let next = rest.chars().next().expect("rest is not empty");
                output.push(next);
                rest = &rest[next.len_utf8()..];
            }
        }
        output
    }

    fn scalar(&mut self, key: Option<&str>, text: String) -> Value {
        let sensitive = key.is_some_and(|key| SENSITIVE_KEYS.contains(&key.to_lowercase().as_str()));
        if sensitive {
            if text.chars().count() >= MIN_SECRET_LENGTH {
                self.secrets.insert(text);
            }
            return Value::String(REDACTED.to_owned());
        }
        if key == Some("private") {
            if let Some(inner) = decode_private(&text) {
                let clean = self.value(None, inner);
                return Value::String(STANDARD.encode(clean.to_string()));
            }
        }
        if text.len() > MAX_OPAQUE_LENGTH && !text.contains(' ') {
            return Value::String(REDACTED.to_owned());
        }
        Value::String(self.text(&text))
    }

    pub fn value(&mut self, key: Option<&str>, value: Value) -> Value {
        match value {
            Value::String(text) => self.scalar(key, text),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|item| self.value(key, item)).collect())
            }
            Value::Object(entries) => {
                let mut output = Map::new();
                for (name, inner) in entries {
                    let clean = self.value(Some(&name), inner);
                    output.insert(self.text(&name), clean);
                }
                Value::Object(output)
            }
            other => other,
        }
    }

    /// 消した値が出力に残っていないか調べる。残っていれば `false`。
    pub fn verify(&self, output: &str) -> bool {
        let lower = output.to_lowercase();
        self.pseudonyms.keys().all(|uuid| !lower.contains(uuid.as_str()))
            && self.secrets.iter().all(|secret| !output.contains(secret.as_str()))
    }
}

fn decode_private(text: &str) -> Option<Value> {
    serde_json::from_slice(&STANDARD.decode(text).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PLAYER: &str = "a1b2c3d4-0000-4000-8000-1234567890ab";
    const AGENT: &str = "add6443a-41bd-e414-f6ad-e58d267f4e95";

    fn anonymizer() -> Anonymizer {
        Anonymizer::new([AGENT.to_owned()])
    }

    #[test]
    fn replaces_ids_consistently_and_keeps_public_ids() {
        let mut a = anonymizer();
        let out = a.value(None, json!({
            "subject": PLAYER, "killer": PLAYER.to_uppercase(), "characterId": AGENT,
            PLAYER: 1, "cid": format!("{PLAYER}@ares"),
        }));
        let fake = out["subject"].as_str().unwrap();
        assert_ne!(fake, PLAYER);
        assert_eq!(out["killer"], fake);
        assert_eq!(out["characterId"], AGENT);
        assert_eq!(out[fake], 1);
        assert_eq!(out["cid"], format!("{fake}@ares"));
    }

    #[test]
    fn pseudonyms_stay_uuid_shaped() {
        let mut a = anonymizer();
        let out = a.value(None, json!(PLAYER));
        assert!(is_uuid(out.as_str().unwrap()));
    }

    #[test]
    fn removes_names_and_tokens_at_any_depth() {
        let mut a = anonymizer();
        let out = a.value(None, json!({"players": [
            {"gameName": "Tanuki", "tagLine": "jp1", "stats": {"kills": 3}},
            {"accessToken": "abc", "GameName": "Lantern"}
        ]}));
        let text = out.to_string();
        assert!(!text.contains("Tanuki") && !text.contains("Lantern") && !text.contains("abc"));
        assert_eq!(out["players"][0]["stats"]["kills"], 3);
    }

    #[test]
    fn cleans_inside_base64_private_presence() {
        let inner = json!({"partyId": PLAYER, "customGameName": "SecretLobby", "partySize": 2});
        let mut a = anonymizer();
        let out = a.value(None, json!({"private": STANDARD.encode(inner.to_string())}));
        let decoded = decode_private(out["private"].as_str().unwrap()).unwrap();
        assert_ne!(decoded["partyId"], PLAYER);
        assert_eq!(decoded["customGameName"], REDACTED);
        assert_eq!(decoded["partySize"], 2);
    }

    #[test]
    fn long_opaque_strings_are_dropped() {
        let mut a = anonymizer();
        let out = a.value(Some("mystery"), json!("x".repeat(MAX_OPAQUE_LENGTH + 1)));
        assert_eq!(out, REDACTED);
    }

    #[test]
    fn verify_detects_values_left_in_the_output() {
        let mut a = anonymizer();
        let clean = a.value(None, json!({"subject": PLAYER, "gameName": "Tanuki"})).to_string();
        assert!(a.verify(&clean));
        assert!(!a.verify(&format!("{clean} {PLAYER}")));
        assert!(!a.verify(&format!("{clean} {}", PLAYER.to_uppercase())));
        assert!(!a.verify(&format!("{clean} Tanuki")));
    }

    #[test]
    fn multibyte_text_survives() {
        let mut a = anonymizer();
        assert_eq!(a.value(Some("note2"), json!("日本語のマップ")), "日本語のマップ");
    }
}
