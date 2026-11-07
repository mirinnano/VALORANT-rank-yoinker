//! ストアとウォレットの表示データ。読み取りだけで、購入の処理は持たない。

use crate::error::Result;
use crate::images::{list, media_url};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const VP: &str = "85ad13f7-3d1b-5128-9eb2-7cd8ee0b5741";
const RADIANITE: &str = "e59aa87c-4cbf-517a-5983-6e81511be9b7";
const KINGDOM: &str = "85ca954a-41f2-ce94-9b45-8ca3dd39a00d";
const SEARCH_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TierView {
    pub name: String,
    /// `#rrggbb`。
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skin {
    pub item_id: String,
    pub name: String,
    pub image: Option<String>,
    pub tier: Option<TierView>,
}

#[derive(Debug, Clone, Default)]
pub struct ShopCatalog {
    pub skins: HashMap<String, Skin>,
    pub bundles: HashMap<String, (String, Option<String>)>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn tier_view(tier: &Value) -> Option<(String, TierView)> {
    let color = text(tier, "highlightColor")?;
    let color = color.get(..6).filter(|hex| hex.chars().all(|c| c.is_ascii_hexdigit()))?;
    let view = TierView { name: text(tier, "devName")?, color: format!("#{color}") };
    Some((text(tier, "uuid")?.to_lowercase(), view))
}

fn skin_entries(skin: &Value, tiers: &HashMap<String, TierView>) -> Vec<Skin> {
    let name = text(skin, "displayName").unwrap_or_default();
    let tier = text(skin, "contentTierUuid").and_then(|id| tiers.get(&id.to_lowercase()).cloned());
    let fallback = media_url(skin, "displayIcon");
    let chroma = skin
        .get("chromas")
        .and_then(Value::as_array)
        .and_then(|chromas| chromas.first())
        .and_then(|chroma| media_url(chroma, "fullRender"));
    skin.get("levels")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|level| {
            Some(Skin {
                item_id: text(level, "uuid")?.to_lowercase(),
                name: name.clone(),
                image: media_url(level, "displayIcon").or(fallback.clone()).or(chroma.clone()),
                tier: tier.clone(),
            })
        })
        .collect()
}

pub fn parse_catalog(skins: &[Value], tiers: &[Value], bundles: &[Value]) -> ShopCatalog {
    let tiers: HashMap<String, TierView> = tiers.iter().filter_map(tier_view).collect();
    ShopCatalog {
        skins: skins
            .iter()
            .flat_map(|skin| skin_entries(skin, &tiers))
            .map(|skin| (skin.item_id.clone(), skin))
            .collect(),
        bundles: bundles
            .iter()
            .filter_map(|bundle| {
                let image = media_url(bundle, "displayIcon");
                Some((text(bundle, "uuid")?.to_lowercase(), (text(bundle, "displayName")?, image)))
            })
            .collect(),
    }
}

pub async fn fetch_catalog() -> Result<ShopCatalog> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let (skins, tiers, bundles) = tokio::try_join!(
        list(&client, "/weapons/skins?language=ja-JP"),
        list(&client, "/contenttiers"),
        list(&client, "/bundles?language=ja-JP"),
    )?;
    Ok(parse_catalog(&skins, &tiers, &bundles))
}

impl ShopCatalog {
    /// 名前の部分一致。購入できるスキンの本体だけを返す。
    pub fn search(&self, query: &str) -> Vec<Skin> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<&Skin> = self
            .skins
            .values()
            .filter(|skin| skin.tier.is_some() && skin.name.to_lowercase().contains(&query))
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.item_id.cmp(&b.item_id)));
        found.dedup_by(|a, b| a.name == b.name);
        found.into_iter().take(SEARCH_LIMIT).cloned().collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Wallet {
    pub vp: u32,
    pub radianite: u32,
    pub kingdom: u32,
}

/// 過去にストアへ出た記録。今回の出現は含めない。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SeenRecord {
    pub times: u32,
    pub last_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShopItem {
    #[serde(flatten)]
    pub skin: Skin,
    pub price: Option<u32>,
    pub base_price: Option<u32>,
    pub discount_percent: Option<u32>,
    pub wished: bool,
    pub seen: SeenRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShopBundle {
    pub name: String,
    pub image: Option<String>,
    pub price: Option<u32>,
    pub items: u32,
    pub remaining_s: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ShopView {
    pub wallet: Wallet,
    pub daily: Vec<ShopItem>,
    pub daily_remaining_s: u64,
    pub bundles: Vec<ShopBundle>,
    pub night_market: Vec<ShopItem>,
    pub night_remaining_s: Option<u64>,
}

fn path<'a>(root: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(root, |value, key| value.get(key))
}

fn cost(costs: Option<&Value>) -> Option<u32> {
    costs?.get(VP)?.as_u64().map(|price| price as u32)
}

fn offer_skin(offer: &Value) -> Option<String> {
    let reward = offer.get("Rewards")?.as_array()?.first()?;
    Some(reward.get("ItemID")?.as_str()?.to_lowercase())
}

pub struct Personal<'a> {
    pub wishlist: &'a HashSet<String>,
    pub seen: &'a HashMap<String, SeenRecord>,
}

fn item(id: String, catalog: &ShopCatalog, personal: &Personal<'_>) -> ShopItem {
    let skin = catalog.skins.get(&id).cloned().unwrap_or(Skin {
        item_id: id.clone(),
        name: "不明なスキン".to_owned(),
        image: None,
        tier: None,
    });
    ShopItem {
        skin,
        price: None,
        base_price: None,
        discount_percent: None,
        wished: personal.wishlist.contains(&id),
        seen: personal.seen.get(&id).copied().unwrap_or_default(),
        }
}

fn seconds(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// 今回のストアに出ているスキンの ID。
pub fn offered_ids(storefront: &Value) -> Vec<String> {
    let daily = path(storefront, &["SkinsPanelLayout", "SingleItemStoreOffers"]);
    let bonus = path(storefront, &["BonusStore", "BonusStoreOffers"]);
    let offers = |list: Option<&Value>| list.and_then(Value::as_array).cloned().unwrap_or_default();
    offers(daily)
        .iter()
        .filter_map(offer_skin)
        .chain(offers(bonus).iter().filter_map(|bonus| offer_skin(bonus.get("Offer")?)))
        .collect()
}

pub fn build(storefront: &Value, wallet: &Value, catalog: &ShopCatalog, personal: &Personal<'_>) -> ShopView {
    let balance = |id: &str| {
        path(wallet, &["Balances", id]).and_then(Value::as_u64).unwrap_or(0) as u32
    };
    let list = |value: Option<&Value>| value.and_then(Value::as_array).cloned().unwrap_or_default();

    let daily = list(path(storefront, &["SkinsPanelLayout", "SingleItemStoreOffers"]))
        .iter()
        .filter_map(|offer| {
            let mut entry = item(offer_skin(offer)?, catalog, personal);
            entry.price = cost(offer.get("Cost"));
            Some(entry)
        })
        .collect();
    let night_market = list(path(storefront, &["BonusStore", "BonusStoreOffers"]))
        .iter()
        .filter_map(|bonus| {
            let offer = bonus.get("Offer")?;
            let mut entry = item(offer_skin(offer)?, catalog, personal);
            entry.base_price = cost(offer.get("Cost"));
            entry.price = cost(bonus.get("DiscountCosts"));
            entry.discount_percent = bonus.get("DiscountPercent").and_then(Value::as_u64).map(|p| p as u32);
            Some(entry)
        })
        .collect();
    let bundles = list(path(storefront, &["FeaturedBundle", "Bundles"]))
        .iter()
        .map(|bundle| {
            let id = bundle.get("DataAssetID").and_then(Value::as_str).unwrap_or_default().to_lowercase();
            let (name, image) = catalog.bundles.get(&id).cloned().unwrap_or(("バンドル".to_owned(), None));
            ShopBundle {
                name,
                image,
                price: cost(bundle.get("TotalDiscountedCost")),
                items: bundle.get("Items").and_then(Value::as_array).map_or(0, |items| items.len() as u32),
                remaining_s: seconds(bundle.get("DurationRemainingInSeconds")),
            }
        })
        .collect();

    ShopView {
        wallet: Wallet { vp: balance(VP), radianite: balance(RADIANITE), kingdom: balance(KINGDOM) },
        daily,
        daily_remaining_s: seconds(path(
            storefront,
            &["SkinsPanelLayout", "SingleItemOffersRemainingDurationInSeconds"],
        )),
        bundles,
        night_market,
        night_remaining_s: path(storefront, &["BonusStore", "BonusStoreRemainingDurationInSeconds"])
            .and_then(Value::as_u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> ShopCatalog {
        parse_catalog(
            &[json!({"displayName": "ラッシュ アレス", "contentTierUuid": "T1",
                "displayIcon": "https://media.valorant-api.com/s.png",
                "levels": [{"uuid": "LEVEL-1", "displayIcon": null}, {"uuid": "level-2", "displayIcon": null}]})],
            &[json!({"uuid": "t1", "devName": "Select", "highlightColor": "5a9fe233"})],
            &[json!({"uuid": "b1", "displayName": "Bundle", "displayIcon": "https://media.valorant-api.com/b.png"})],
        )
    }

    fn storefront() -> Value {
        json!({
            "FeaturedBundle": {"Bundles": [{"DataAssetID": "B1", "Items": [{}, {}],
                "TotalDiscountedCost": {(VP): 5000}, "DurationRemainingInSeconds": 90}]},
            "SkinsPanelLayout": {"SingleItemOffersRemainingDurationInSeconds": 60,
                "SingleItemStoreOffers": [{"Cost": {(VP): 875}, "Rewards": [{"ItemID": "LEVEL-1"}]}]},
            "BonusStore": {"BonusStoreRemainingDurationInSeconds": 120, "BonusStoreOffers": [{
                "Offer": {"Cost": {(VP): 1775}, "Rewards": [{"ItemID": "unknown"}]},
                "DiscountCosts": {(VP): 900}, "DiscountPercent": 49}]}
        })
    }

    #[test]
    fn builds_daily_bundle_night_market_and_wallet() {
        let wished = HashSet::from(["level-1".to_owned()]);
        let seen = HashMap::from([("level-1".to_owned(), SeenRecord { times: 2, last_ms: Some(9) })]);
        let personal = Personal { wishlist: &wished, seen: &seen };
        let wallet = json!({"Balances": {(VP): 1200, (RADIANITE): 30, (KINGDOM): 7}});
        let shop = build(&storefront(), &wallet, &catalog(), &personal);

        assert_eq!(shop.wallet, Wallet { vp: 1200, radianite: 30, kingdom: 7 });
        let daily = &shop.daily[0];
        assert_eq!(daily.skin.name, "ラッシュ アレス");
        assert_eq!(daily.skin.tier.as_ref().unwrap().color, "#5a9fe2");
        assert_eq!(daily.skin.image.as_deref(), Some("https://media.valorant-api.com/s.png"));
        assert_eq!((daily.price, daily.wished, daily.seen.times), (Some(875), true, 2));
        assert_eq!(shop.bundles[0].name, "Bundle");
        assert_eq!((shop.bundles[0].price, shop.bundles[0].items), (Some(5000), 2));
        let night = &shop.night_market[0];
        assert_eq!((night.price, night.base_price, night.discount_percent), (Some(900), Some(1775), Some(49)));
        assert_eq!(night.skin.name, "不明なスキン");
        assert_eq!(shop.night_remaining_s, Some(120));
    }

    #[test]
    fn no_night_market_is_empty_not_an_error() {
        let none = HashSet::new();
        let seen = HashMap::new();
        let shop = build(&json!({}), &json!({}), &ShopCatalog::default(), &Personal { wishlist: &none, seen: &seen });
        assert_eq!(shop, ShopView::default());
    }

    #[test]
    fn offered_ids_cover_daily_and_night_market() {
        assert_eq!(offered_ids(&storefront()), ["level-1", "unknown"]);
    }

    #[test]
    fn search_matches_part_of_a_name_once() {
        let found = catalog().search("アレス");
        assert_eq!(found.len(), 1);
        assert!(catalog().search("  ").is_empty());
    }

    #[test]
    fn colors_that_are_not_hex_are_dropped() {
        assert_eq!(tier_view(&json!({"uuid": "a", "devName": "x", "highlightColor": "url(x)"})), None);
    }
}
