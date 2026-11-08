//! 画面で使う画像の URL。公開データから取得する。
//!
//! 外部の応答をそのまま UI へ渡さないよう、想定したホストの URL だけを受け取る。

use crate::error::{Error, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const API: &str = "https://valorant-api.com/v1";
const MEDIA_PREFIX: &str = "https://media.valorant-api.com/";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentImages {
    pub icon: Option<String>,
    pub portrait: Option<String>,
    /// Duelist などの役割。
    pub role: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MapImages {
    pub splash: Option<String>,
    pub thumbnail: Option<String>,
    /// 上から見たミニマップ。
    pub minimap: Option<String>,
    /// ゲーム内の座標をミニマップの 0〜1 へ変換する値。
    /// `u = y * x_multiplier + x_scalar`、`v = x * y_multiplier + y_scalar`。
    pub transform: Option<MapTransform>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct MapTransform {
    pub x_multiplier: f64,
    pub y_multiplier: f64,
    pub x_scalar: f64,
    pub y_scalar: f64,
}

fn transform(map: &Value) -> Option<MapTransform> {
    let number = |key| map.get(key).and_then(Value::as_f64);
    let transform = MapTransform {
        x_multiplier: number("xMultiplier")?,
        y_multiplier: number("yMultiplier")?,
        x_scalar: number("xScalarToAdd")?,
        y_scalar: number("yScalarToAdd")?,
    };
    // 練習場などは 0 が入っていて、変換に使えない。
    (transform.x_multiplier != 0.0 && transform.y_multiplier != 0.0).then_some(transform)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TierImages {
    pub small: Option<String>,
    pub large: Option<String>,
}

/// 表示名と tier から引く。UI の値をそのままキーにできるようにする。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImageCatalog {
    pub agents: HashMap<String, AgentImages>,
    pub maps: HashMap<String, MapImages>,
    pub tiers: HashMap<u32, TierImages>,
    /// 称号の ID から文言を引く。
    pub titles: HashMap<String, String>,
}

pub(crate) fn media_url(value: &Value, key: &str) -> Option<String> {
    let url = value.get(key)?.as_str()?;
    url.starts_with(MEDIA_PREFIX).then(|| url.to_owned())
}

fn name(value: &Value) -> Option<String> {
    Some(value.get("displayName")?.as_str()?.to_owned())
}

pub fn parse(agents: &[Value], maps: &[Value], tier_sets: &[Value], titles: &[Value]) -> ImageCatalog {
    let tiers = tier_sets
        .last()
        .and_then(|set| set.get("tiers"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    ImageCatalog {
        agents: agents
            .iter()
            .filter_map(|agent| {
                let images = AgentImages {
                    icon: media_url(agent, "displayIcon"),
                    portrait: media_url(agent, "fullPortrait"),
                    role: agent
                        .get("role")
                        .and_then(|role| role.get("displayName"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                };
                Some((name(agent)?, images))
            })
            .collect(),
        maps: maps
            .iter()
            .filter_map(|map| {
                let images = MapImages {
                    splash: media_url(map, "splash"),
                    thumbnail: media_url(map, "listViewIcon"),
                    minimap: media_url(map, "displayIcon"),
                    transform: transform(map),
                };
                Some((name(map)?, images))
            })
            .collect(),
        tiers: tiers
            .iter()
            .filter_map(|tier| {
                let number = u32::try_from(tier.get("tier")?.as_u64()?).ok()?;
                let images = TierImages {
                    small: media_url(tier, "smallIcon"),
                    large: media_url(tier, "largeIcon"),
                };
                Some((number, images))
            })
            .collect(),
        titles: titles
            .iter()
            .filter_map(|title| {
                let id = title.get("uuid")?.as_str()?.to_lowercase();
                Some((id, title.get("titleText")?.as_str()?.to_owned()))
            })
            .collect(),
    }
}

pub(crate) async fn list(client: &reqwest::Client, path: &str) -> Result<Vec<Value>> {
    let value: Value =
        client.get(format!("{API}{path}")).send().await?.error_for_status()?.json().await?;
    match value.get("data") {
        Some(Value::Array(items)) => Ok(items.clone()),
        _ => Err(Error::Decode(format!("valorant-api{path}"))),
    }
}

pub async fn fetch() -> Result<ImageCatalog> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let (agents, maps, tiers, titles) = tokio::try_join!(
        list(&client, "/agents?isPlayableCharacter=true"),
        list(&client, "/maps"),
        list(&client, "/competitivetiers"),
        list(&client, "/playertitles"),
    )?;
    Ok(parse(&agents, &maps, &tiers, &titles))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ICON: &str = "https://media.valorant-api.com/agents/a/displayicon.png";

    #[test]
    fn reads_agents_maps_and_the_latest_tier_set() {
        let agents = [json!({"displayName": "Jett", "displayIcon": ICON, "fullPortrait": null})];
        let maps = [json!({"displayName": "Ascent", "splash": "https://media.valorant-api.com/m/s.png"})];
        let tiers = [
            json!({"tiers": [{"tier": 15, "smallIcon": "https://media.valorant-api.com/old.png"}]}),
            json!({"tiers": [{"tier": 15, "smallIcon": "https://media.valorant-api.com/new.png"}]}),
        ];
        let catalog = parse(&agents, &maps, &tiers, &[]);
        assert_eq!(catalog.agents["Jett"].icon.as_deref(), Some(ICON));
        assert_eq!(catalog.agents["Jett"].portrait, None);
        assert!(catalog.maps["Ascent"].splash.is_some());
        assert_eq!(catalog.tiers[&15].small.as_deref(), Some("https://media.valorant-api.com/new.png"));
    }

    #[test]
    fn rejects_urls_from_other_hosts() {
        let agents = [json!({
            "displayName": "Jett",
            "displayIcon": "https://evil.example/x.png",
            "fullPortrait": "javascript:alert(1)"
        })];
        let catalog = parse(&agents, &[], &[], &[]);
        assert_eq!(catalog.agents["Jett"], AgentImages::default());
    }

    #[test]
    fn a_look_alike_host_is_rejected() {
        let value = json!({"u": "https://media.valorant-api.com.evil.example/x.png"});
        assert_eq!(media_url(&value, "u"), None);
    }

    #[test]
    fn empty_data_is_an_empty_catalog() {
        assert_eq!(parse(&[], &[], &[], &[]), ImageCatalog::default());
    }
}
