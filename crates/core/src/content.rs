//! シーズン情報の解析。

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Season {
    #[serde(rename = "ID")]
    pub id: String,
    pub name: String,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub is_active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Seasons {
    list: Vec<Season>,
}

impl Seasons {
    pub fn from_content(content: &Value) -> Result<Self, serde_json::Error> {
        let list = content.get("Seasons").cloned().unwrap_or(Value::Array(Vec::new()));
        Ok(Self { list: serde_json::from_value(list)? })
    }

    fn current_act(&self) -> Option<&Season> {
        self.list.iter().rev().find(|s| s.is_active && s.kind == "act")
    }

    pub fn current_id(&self) -> Option<&str> {
        self.current_act().map(|s| s.id.as_str())
    }

    pub fn previous_id(&self) -> Option<&str> {
        let start = self.current_act()?.start_time.as_ref()?;
        self.list
            .iter()
            .find(|s| s.kind == "act" && s.end_time.as_ref() == Some(start))
            .map(|s| s.id.as_str())
    }

    /// `e9a2` や `v25a3` の形式で幕の名前を返す。
    pub fn act_label(&self, act_id: &str) -> Option<String> {
        let index = self.list.iter().position(|s| s.id.eq_ignore_ascii_case(act_id))?;
        let act = parse_number(&self.list[index].name)?;
        let episode = self.list[..index]
            .iter()
            .rev()
            .find(|s| s.kind == "episode")
            .and_then(|s| parse_number(&s.name))?;
        Some(match episode.chars().any(char::is_alphabetic) {
            true => format!("{episode}a{act}"),
            false => format!("e{episode}a{act}"),
        })
    }
}

fn roman_to_int(text: &str) -> Option<u32> {
    let value = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        _ => None,
    };
    let mut total = 0i32;
    let mut previous = 0;
    for c in text.to_ascii_uppercase().chars().rev() {
        let current = value(c)?;
        total += if current < previous { -current } else { current };
        previous = previous.max(current);
    }
    u32::try_from(total).ok().filter(|n| *n > 0)
}

fn parse_number(name: &str) -> Option<String> {
    let last = name.split_whitespace().last()?;
    let has_letter = last.chars().any(char::is_alphabetic);
    let has_digit = last.chars().any(|c| c.is_ascii_digit());
    if has_letter && has_digit {
        return Some(last.to_lowercase());
    }
    last.parse::<u32>().ok().or_else(|| roman_to_int(last)).map(|n| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seasons() -> Seasons {
        Seasons::from_content(&json!({"Seasons": [
            {"ID": "ep8", "Name": "EPISODE 8", "Type": "episode"},
            {"ID": "a1", "Name": "ACT I", "Type": "act", "EndTime": "t1"},
            {"ID": "a2", "Name": "ACT II", "Type": "act", "StartTime": "t1", "EndTime": "t2"},
            {"ID": "v25", "Name": "V25", "Type": "episode"},
            {"ID": "a3", "Name": "ACT 3", "Type": "act", "StartTime": "t2", "IsActive": true}
        ]}))
        .unwrap()
    }

    #[test]
    fn finds_current_and_previous() {
        let s = seasons();
        assert_eq!(s.current_id(), Some("a3"));
        assert_eq!(s.previous_id(), Some("a2"));
    }

    #[test]
    fn labels_acts() {
        let s = seasons();
        assert_eq!(s.act_label("A2").as_deref(), Some("e8a2"));
        // 元実装は最後のエピソードの幕でエピソード番号を失っていた。
        assert_eq!(s.act_label("a3").as_deref(), Some("v25a3"));
        assert_eq!(s.act_label("missing"), None);
    }

    #[test]
    fn roman_numerals() {
        assert_eq!(roman_to_int("IV"), Some(4));
        assert_eq!(roman_to_int("IX"), Some(9));
        assert_eq!(roman_to_int("XIV"), Some(14));
        assert_eq!(roman_to_int("ABC"), None);
    }
}
