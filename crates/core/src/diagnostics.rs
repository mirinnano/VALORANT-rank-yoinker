//! 実機で動かすための診断。
//!
//! 各 API の成否と、応答に必要な項目があるかを調べる。レポートには項目名と結果だけを
//! 入れるので、名前や puuid は含まれない。

use crate::anonymize::Anonymizer;
use crate::content::Seasons;
use crate::error::{Error, Result};
use crate::model::GameState;
use crate::presence::{self, Presence};
use crate::replay::Scenario;
use crate::review;
use crate::roster;
use crate::source::RiotSource;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

const HISTORY_MATCHES: u32 = 20;
const RECORDED_MATCHES: usize = 5;
const RECORDED_NAME: &str = "Recorded#0000";

/// `Confirmed` は元の vRY で使われている項目。`Unconfirmed` は実機で未確認の項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trust {
    Confirmed,
    Unconfirmed,
}
use Trust::{Confirmed, Unconfirmed};

type Field = (&'static str, Trust);

const MMR_FIELDS: [Field; 6] = [
    ("QueueSkills.competitive.SeasonalInfoBySeasonID", Confirmed),
    ("QueueSkills.competitive.SeasonalInfoBySeasonID.*.CompetitiveTier", Confirmed),
    ("QueueSkills.competitive.SeasonalInfoBySeasonID.*.RankedRating", Confirmed),
    ("QueueSkills.competitive.SeasonalInfoBySeasonID.*.NumberOfGames", Confirmed),
    ("QueueSkills.competitive.SeasonalInfoBySeasonID.*.NumberOfWinsWithPlacements", Confirmed),
    ("QueueSkills.competitive.SeasonalInfoBySeasonID.*.WinsByTier", Confirmed),
];
const HISTORY_FIELDS: [Field; 6] = [
    ("Matches.*.MatchID", Confirmed),
    ("Matches.*.RankedRatingEarned", Confirmed),
    ("Matches.*.MatchStartTime", Confirmed),
    ("Matches.*.MapID", Unconfirmed),
    ("Matches.*.TierAfterUpdate", Unconfirmed),
    ("Matches.*.RankedRatingAfterUpdate", Unconfirmed),
];
const MATCH_FIELDS: [Field; 15] = [
    ("matchInfo.gameStartMillis", Confirmed),
    ("matchInfo.gameLengthMillis", Confirmed),
    ("players.*.subject", Confirmed),
    ("players.*.stats.kills", Confirmed),
    ("players.*.stats.deaths", Confirmed),
    ("roundResults.*.playerStats.*.subject", Confirmed),
    ("roundResults.*.playerStats.*.damage.*.headshots", Confirmed),
    ("players.*.teamId", Unconfirmed),
    ("players.*.characterId", Unconfirmed),
    ("players.*.competitiveTier", Unconfirmed),
    ("players.*.stats.score", Unconfirmed),
    ("players.*.stats.assists", Unconfirmed),
    ("roundResults.*.winningTeam", Unconfirmed),
    ("roundResults.*.playerStats.*.damage.*.damage", Unconfirmed),
    ("roundResults.*.playerStats.*.economy.loadoutValue", Unconfirmed),
];
const ANALYTICS_FIELDS: [Field; 5] = [
    ("roundResults.*.playerStats.*.damage.*.receiver", Unconfirmed),
    ("roundResults.*.playerStats.*.economy.spent", Unconfirmed),
    ("roundResults.*.playerStats.*.kills.*.assistants", Unconfirmed),
    ("roundResults.*.bombPlanter", Unconfirmed),
    ("roundResults.*.playerStats.*.economy.loadoutValue", Unconfirmed),
];
const KILL_TIME_FIELDS: [&str; 2] = [
    "roundResults.*.playerStats.*.kills.*.timeSinceRoundStartMillis",
    "roundResults.*.playerStats.*.kills.*.roundTime",
];
const PRESENCE_FIELDS: [Field; 5] = [
    ("sessionLoopState", Confirmed),
    ("queueId", Confirmed),
    ("partyId", Confirmed),
    ("partySize", Confirmed),
    ("provisioningFlow", Confirmed),
];
const LIVE_SCORE_FIELDS: [Field; 2] = [
    ("partyOwnerMatchScoreAllyTeam", Unconfirmed),
    ("partyOwnerMatchScoreEnemyTeam", Unconfirmed),
];
const PREGAME_FIELDS: [Field; 6] = [
    ("PhaseTimeRemainingNS", Unconfirmed),
    ("ID", Confirmed),
    ("AllyTeam.Players.*.Subject", Confirmed),
    ("AllyTeam.Players.*.CharacterID", Confirmed),
    ("AllyTeam.Players.*.CharacterSelectionState", Confirmed),
    ("AllyTeam.Players.*.PlayerIdentity.Incognito", Confirmed),
];
const COREGAME_FIELDS: [Field; 5] = [
    ("MapID", Confirmed),
    ("Players.*.Subject", Confirmed),
    ("Players.*.TeamID", Confirmed),
    ("Players.*.CharacterID", Confirmed),
    ("Players.*.PlayerIdentity.HideAccountLevel", Confirmed),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub label: &'static str,
    pub status: CheckStatus,
    pub detail: String,
    /// 応答になかった項目。
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    /// 共有しやすい文章にする。項目名と結果だけを含む。
    pub fn to_text(&self) -> String {
        let mut text = format!("vRY 診断 v{}\n", env!("CARGO_PKG_VERSION"));
        for check in &self.checks {
            let status = match check.status {
                CheckStatus::Pass => "OK",
                CheckStatus::Warn => "WARN",
                CheckStatus::Fail => "FAIL",
                CheckStatus::Skip => "SKIP",
            };
            text.push_str(&format!("[{status}] {}: {}\n", check.label, check.detail));
            for field in &check.missing {
                text.push_str(&format!("    missing {field}\n"));
            }
        }
        text
    }
}

/// パスの項目があるか調べる。`*` は配列か辞書のいずれかの要素に一致する。
fn has_path(value: &Value, path: &[&str]) -> bool {
    let Some((head, rest)) = path.split_first() else { return !value.is_null() };
    if *head == "*" {
        return match value {
            Value::Array(items) => items.iter().any(|item| has_path(item, rest)),
            Value::Object(entries) => entries.values().any(|item| has_path(item, rest)),
            _ => false,
        };
    }
    value.get(*head).is_some_and(|inner| has_path(inner, rest))
}

pub fn check(label: &'static str, status: CheckStatus, detail: impl Into<String>) -> Check {
    Check { label, status, detail: detail.into(), missing: Vec::new() }
}

fn skipped(label: &'static str, detail: &str) -> Check {
    check(label, CheckStatus::Skip, detail)
}

/// 欠けた項目から結果を決める。確認済みの項目が欠ければ失敗、未確認なら警告。
fn judge<F>(label: &'static str, fields: &[Field], exists: F) -> Check
where
    F: Fn(&str) -> bool,
{
    let missing: Vec<&Field> = fields.iter().filter(|(path, _)| !exists(path)).collect();
    let status = if missing.iter().any(|(_, trust)| *trust == Confirmed) {
        CheckStatus::Fail
    } else if missing.is_empty() {
        CheckStatus::Pass
    } else {
        CheckStatus::Warn
    };
    Check {
        label,
        status,
        detail: format!("{} 項目中 {} 項目を確認", fields.len(), fields.len() - missing.len()),
        missing: missing.iter().map(|(path, _)| (*path).to_owned()).collect(),
    }
}

fn judge_value(label: &'static str, fields: &[Field], value: &Value) -> Check {
    judge(label, fields, |path| has_path(value, &path.split('.').collect::<Vec<_>>()))
}

fn failed(label: &'static str, error: &Error) -> Check {
    check(label, CheckStatus::Fail, error.to_string())
}

/// 応答を項目検査する。応答がなければ `absent` の理由で飛ばす。
fn judge_reply(
    label: &'static str,
    fields: &[Field],
    reply: &Result<Option<Value>>,
    absent: &str,
) -> Check {
    match reply {
        Ok(Some(value)) => judge_value(label, fields, value),
        Ok(None) => skipped(label, absent),
        Err(error) => failed(label, error),
    }
}

fn check_presence(presences: &[Presence], own: &str, checks: &mut Vec<Check>) -> Option<GameState> {
    const LABEL: &str = "自分のプレゼンス";
    let Some(private) = presences.iter().find(|p| p.puuid == own).and_then(Presence::private_value)
    else {
        checks.push(check(LABEL, CheckStatus::Fail, "自分のプレゼンスがありません"));
        return None;
    };
    let exists = |key: &str| presence::has_key(&private, key);
    checks.push(judge(LABEL, &PRESENCE_FIELDS, exists));

    let state = presence::find_own(presences, own).and_then(|own| own.state);
    if state == Some(GameState::InGame) {
        checks.push(judge("進行中のスコア", &LIVE_SCORE_FIELDS, exists));
    } else {
        checks.push(skipped("進行中のスコア", "試合中に実行すると確認できます"));
    }
    state
}

fn check_review(own: &str, details: &Value, source_catalog: &crate::source::GameCatalog, checks: &mut Vec<Check>) {
    let mut schema = judge_value("試合詳細", &MATCH_FIELDS, details);
    let has_kill_time =
        KILL_TIME_FIELDS.iter().any(|path| has_path(details, &path.split('.').collect::<Vec<_>>()));
    if !has_kill_time {
        schema.missing.push(KILL_TIME_FIELDS[0].to_owned());
        if schema.status == CheckStatus::Pass {
            schema.status = CheckStatus::Warn;
        }
    }
    checks.push(schema);
    checks.push(judge_value("指標の計算", &ANALYTICS_FIELDS, details));

    const LABEL: &str = "振り返りの集計";
    let Some(result) = review::analyze(own, details, source_catalog) else {
        checks.push(check(LABEL, CheckStatus::Fail, "試合詳細に自分のデータがありません"));
        return;
    };
    let rounds = result.rounds.len();
    if rounds == 0 {
        checks.push(check(LABEL, CheckStatus::Warn, "ラウンドを読み取れません"));
        return;
    }
    let detail = format!("{rounds} ラウンド、ACS {}", result.own.acs);
    let status = if result.own.acs == 0 || result.own.adr == 0 { CheckStatus::Warn } else { CheckStatus::Pass };
    checks.push(check(LABEL, status, detail));
}

/// 診断を実行する。途中で失敗しても、確認できる範囲を報告する。
pub async fn run(source: &dyn RiotSource) -> Report {
    let mut checks = Vec::new();

    let own = match source.connect().await {
        Ok(own) => {
            checks.push(check("Riot Client への接続", CheckStatus::Pass, "接続できました"));
            own
        }
        Err(error) => {
            checks.push(failed("Riot Client への接続", &error));
            return Report { checks };
        }
    };

    let state = match source.presences().await {
        Ok(presences) => check_presence(&presences, &own, &mut checks),
        Err(error) => {
            checks.push(failed("自分のプレゼンス", &error));
            None
        }
    };

    match source.content().await {
        Ok(content) => match Seasons::from_content(&content) {
            Ok(seasons) if seasons.current_id().is_some() => {
                let previous = if seasons.previous_id().is_some() { "前シーズンも確認" } else { "前シーズンは不明" };
                checks.push(check("シーズン情報", CheckStatus::Pass, previous));
            }
            Ok(_) => checks.push(check("シーズン情報", CheckStatus::Fail, "現在のシーズンがありません")),
            Err(error) => checks.push(failed("シーズン情報", &error.into())),
        },
        Err(error) => checks.push(failed("シーズン情報", &error)),
    }

    let catalog = match source.catalog().await {
        Ok(catalog) => {
            let detail = format!("エージェント {}、マップ {}", catalog.agents.len(), catalog.maps.len());
            let empty = catalog.agents.is_empty() || catalog.maps.is_empty();
            let status = if empty { CheckStatus::Fail } else { CheckStatus::Pass };
            checks.push(check("公開データ", status, detail));
            catalog
        }
        Err(error) => {
            checks.push(failed("公開データ", &error));
            Default::default()
        }
    };

    checks.push(judge_reply("ランク", &MMR_FIELDS, &source.mmr(&own).await, "ランクの応答がありません"));

    match source.names(std::slice::from_ref(&own)).await {
        Ok(names) if names.contains_key(&own) => {
            checks.push(check("自分の名前", CheckStatus::Pass, "取得できました"));
        }
        Ok(_) => checks.push(check("自分の名前", CheckStatus::Fail, "応答に自分がいません")),
        Err(error) => checks.push(failed("自分の名前", &error)),
    }

    let history = source.rank_history(&own, HISTORY_MATCHES).await;
    let empty = matches!(&history, Ok(Some(value)) if crate::stats::latest_match(value).is_none());
    if empty {
        checks.push(skipped("RR の履歴", "コンペティティブの履歴がありません"));
    } else {
        checks.push(judge_reply("RR の履歴", &HISTORY_FIELDS, &history, "履歴の応答がありません"));
    }

    let latest = history.as_ref().ok().and_then(Option::as_ref).and_then(crate::stats::latest_match);
    let match_id = latest.and_then(|m| m.get("MatchID")).and_then(Value::as_str);
    match match_id {
        Some(id) => match source.match_details(id).await {
            Ok(Some(details)) => check_review(&own, &details, &catalog, &mut checks),
            Ok(None) => checks.push(skipped("試合詳細", "試合の集計がまだ終わっていません")),
            Err(error) => checks.push(failed("試合詳細", &error)),
        },
        None => checks.push(skipped("試合詳細", "調べる試合がありません")),
    }

    if state == Some(GameState::Pregame) {
        let reply = source.pregame(&own).await;
        checks.push(judge_reply("エージェント選択", &PREGAME_FIELDS, &reply, "応答がありません"));
    } else {
        checks.push(skipped("エージェント選択", "エージェント選択中に実行すると確認できます"));
    }
    if state == Some(GameState::InGame) {
        let reply = source.coregame(&own).await;
        checks.push(judge_reply("試合中のデータ", &COREGAME_FIELDS, &reply, "応答がありません"));
    } else {
        checks.push(skipped("試合中のデータ", "試合中に実行すると確認できます"));
    }

    Report { checks }
}

fn players_in(raw: Option<&Value>, parse: fn(&Value) -> Result<roster::RawMatch>) -> Vec<String> {
    raw.and_then(|value| parse(value).ok())
        .map(|parsed| parsed.players.into_iter().map(|player| player.subject).collect())
        .unwrap_or_default()
}

/// 現在の応答を集める。名前は自分の分も仮の値にする。
async fn capture(source: &dyn RiotSource) -> Result<Scenario> {
    let own = source.connect().await?;
    let presences = source.presences().await?;
    let state = presence::find_own(&presences, &own).and_then(|own| own.state);
    let pregame = match state {
        Some(GameState::Pregame) => source.pregame(&own).await?,
        _ => None,
    };
    let coregame = match state {
        Some(GameState::InGame) => source.coregame(&own).await?,
        _ => None,
    };

    let mut players: BTreeSet<String> = BTreeSet::from([own.clone()]);
    players.extend(players_in(pregame.as_ref(), roster::parse_pregame));
    players.extend(players_in(coregame.as_ref(), roster::parse_coregame));

    let mut mmr = HashMap::new();
    let mut updates = HashMap::new();
    for player in &players {
        if let Some(value) = source.mmr(player).await? {
            mmr.insert(player.clone(), value);
        }
        if let Some(value) = source.competitive_updates(player).await? {
            updates.insert(player.clone(), value);
        }
    }

    let history = source.rank_history(&own, HISTORY_MATCHES).await?;
    let ids: BTreeSet<String> = history
        .iter()
        .chain(updates.values())
        .flat_map(|value| value.get("Matches").and_then(Value::as_array).into_iter().flatten())
        .filter_map(|entry| entry.get("MatchID")?.as_str().map(str::to_owned))
        .take(RECORDED_MATCHES)
        .collect();
    let mut matches = HashMap::new();
    for id in ids {
        if let Some(details) = source.match_details(&id).await? {
            matches.insert(id, details);
        }
    }

    let frame = presences.iter().map(serde_json::to_value).collect::<std::result::Result<Vec<_>, _>>()?;
    let content = source.content().await?;
    let seasons = content.get("Seasons").cloned().unwrap_or(Value::Array(Vec::new()));
    let catalog = source.catalog().await?;
    Ok(Scenario {
        names: HashMap::from([(own.clone(), RECORDED_NAME.to_owned())]),
        own,
        timeline: vec![frame],
        pregame,
        coregame,
        mmr,
        updates,
        history,
        fail_history: false,
        matches,
        content: serde_json::json!({ "Seasons": seasons }),
        agents: catalog.agents,
        maps: catalog.maps,
    })
}

fn public_ids(scenario: &Scenario) -> Vec<String> {
    let seasons = scenario.content.get("Seasons").and_then(Value::as_array);
    scenario
        .agents
        .keys()
        .cloned()
        .chain(seasons.into_iter().flatten().filter_map(|s| s.get("ID")?.as_str().map(str::to_owned)))
        .collect()
}

/// 記録を匿名化して JSON にする。元の値が残っていれば保存せずにエラーを返す。
pub fn anonymize_recording(scenario: Scenario) -> Result<String> {
    let mut anonymizer = Anonymizer::new(public_ids(&scenario));
    let names = scenario.names.keys().cloned().collect::<Vec<_>>();
    let mut value = serde_json::to_value(&scenario)?;

    // 公開データは匿名化の対象にしない。
    let object = value.as_object_mut().ok_or_else(|| Error::Decode("記録".into()))?;
    let public: Vec<(String, Value)> = ["content", "agents", "maps"]
        .iter()
        .filter_map(|key| object.remove(*key).map(|inner| ((*key).to_owned(), inner)))
        .collect();

    let mut clean = anonymizer.value(None, value);
    // 名前の検査は公開データを戻す前に行う。`Split` のようなマップ名と同じ名前で誤検知しないため。
    let verified = anonymizer.verify(&serde_json::to_string(&clean)?);
    let object = clean.as_object_mut().ok_or_else(|| Error::Decode("記録".into()))?;
    object.extend(public);

    let text = serde_json::to_string(&clean)?;
    let leaked = names.iter().any(|puuid| text.to_lowercase().contains(&puuid.to_lowercase()));
    if leaked || !verified {
        return Err(Error::Decode("匿名化の検査に失敗したため保存しませんでした".into()));
    }
    Ok(text)
}

/// 現在の応答を匿名化して記録する。
pub async fn record(source: &dyn RiotSource) -> Result<String> {
    anonymize_recording(capture(source).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::engine::Engine;
    use crate::replay::ReplaySource;
    use serde_json::json;
    use std::sync::Arc;

    fn find<'a>(report: &'a Report, label: &str) -> &'a Check {
        report.checks.iter().find(|check| check.label == label).expect(label)
    }

    async fn report(scenario: Scenario) -> Report {
        run(&ReplaySource::new(scenario)).await
    }

    fn realistic_scenario() -> Scenario {
        // demo の ID を実物と同じ UUID 形式へ変える。
        let mut text = serde_json::to_string(&demo::scenario()).unwrap();
        let ids: Vec<String> = demo::scenario().names.keys().cloned().collect();
        for (index, id) in ids.iter().enumerate() {
            text = text.replace(id, &format!("1234abcd-0000-4000-9000-{index:012x}"));
        }
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn has_path_walks_arrays_and_maps() {
        let value = json!({"a": [{"b": 1}, {"c": {"x": {"d": 2}}}]});
        assert!(has_path(&value, &["a", "*", "b"]));
        assert!(has_path(&value, &["a", "*", "c", "*", "d"]));
        assert!(!has_path(&value, &["a", "*", "z"]));
        assert!(!has_path(&json!({"a": null}), &["a"]));
    }

    #[tokio::test]
    async fn demo_data_passes_every_available_check() {
        let report = report(demo::scenario()).await;
        let problems: Vec<_> = report
            .checks
            .iter()
            .filter(|check| matches!(check.status, CheckStatus::Fail | CheckStatus::Warn))
            .collect();
        assert!(problems.is_empty(), "{problems:#?}");
        assert_eq!(find(&report, "エージェント選択").status, CheckStatus::Skip);
    }

    #[tokio::test]
    async fn missing_confirmed_field_fails_and_is_named() {
        let mut scenario = demo::scenario();
        let own = demo::OWN.to_owned();
        scenario.mmr.insert(own, json!({"QueueSkills": {}}));
        let report = report(scenario).await;
        let rank = find(&report, "ランク");
        assert_eq!(rank.status, CheckStatus::Fail);
        assert!(rank.missing[0].contains("SeasonalInfoBySeasonID"));
    }

    #[tokio::test]
    async fn missing_unconfirmed_field_only_warns() {
        let mut scenario = demo::scenario();
        for entry in scenario.history.as_mut().unwrap()["Matches"].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("TierAfterUpdate");
        }
        let report = report(scenario).await;
        let history = find(&report, "RR の履歴");
        assert_eq!(history.status, CheckStatus::Warn);
        assert_eq!(history.missing, ["Matches.*.TierAfterUpdate"]);
    }

    #[tokio::test]
    async fn live_score_is_checked_only_in_game() {
        let mut scenario = demo::scenario();
        let frame = scenario.timeline.last().unwrap().clone();
        scenario.timeline = vec![frame];
        let report = report(scenario).await;
        assert_eq!(find(&report, "進行中のスコア").status, CheckStatus::Pass);
        assert_eq!(find(&report, "試合中のデータ").status, CheckStatus::Pass);
    }

    #[tokio::test]
    async fn no_competitive_history_is_skipped_not_failed() {
        let mut scenario = demo::scenario();
        scenario.history = Some(json!({"Matches": []}));
        let report = report(scenario).await;
        assert_eq!(find(&report, "RR の履歴").status, CheckStatus::Skip);
        assert_eq!(find(&report, "試合詳細").status, CheckStatus::Skip);
    }

    #[tokio::test]
    async fn report_text_contains_no_identity() {
        let scenario = realistic_scenario();
        let ids: Vec<String> = scenario.names.keys().cloned().collect();
        let text = report(scenario).await.to_text();
        assert!(text.contains("[OK]"));
        assert!(ids.iter().all(|id| !text.contains(id.as_str())));
        assert!(!text.contains("Tanuki") && !text.contains("You#0000"));
    }

    #[tokio::test]
    async fn recording_hides_identity_and_replays() {
        let mut scenario = realistic_scenario();
        let frame = scenario.timeline.last().unwrap().clone();
        scenario.timeline = vec![frame];
        let ids: Vec<String> = scenario.names.keys().cloned().collect();

        let text = record(&ReplaySource::new(scenario)).await.unwrap();
        assert!(ids.iter().all(|id| !text.contains(id.as_str())), "puuid が残っている");
        for name in ["Tanuki", "You#0000", "Lantern", "HiddenName", "SecretName"] {
            assert!(!text.contains(name), "{name} が残っている");
        }

        let replayed: Scenario = serde_json::from_str(&text).unwrap();
        let mut engine = Engine::new(Arc::new(ReplaySource::new(replayed)), None);
        let snapshot = engine.step().await.unwrap();
        assert_eq!(snapshot.players.len(), 10);
        assert!(snapshot.players.iter().any(|player| player.rank.tier > 0));
        assert_eq!(snapshot.map.as_deref(), Some("Ascent"));
    }

    #[test]
    fn a_player_named_like_a_map_can_still_be_saved() {
        let own = "1234abcd-0000-4000-9000-000000000000".to_owned();
        let scenario = Scenario {
            names: HashMap::from([(own.clone(), RECORDED_NAME.to_owned())]),
            own,
            matches: HashMap::from([("m".to_owned(), json!({"players": [{"gameName": "Split"}]}))]),
            maps: HashMap::from([("/game/maps/bonsai".to_owned(), "Split".to_owned())]),
            ..Default::default()
        };
        let text = anonymize_recording(scenario).unwrap();
        assert!(text.contains("redacted"));
    }

    #[test]
    fn saving_is_refused_when_a_value_survives() {
        let own = "1234abcd-0000-4000-9000-000000000000".to_owned();
        let scenario = Scenario {
            names: HashMap::from([(own.clone(), RECORDED_NAME.to_owned())]),
            // 公開データとして残る欄に puuid が紛れた場合を再現する。
            maps: HashMap::from([("leak".to_owned(), own)]),
            ..Default::default()
        };
        assert!(anonymize_recording(scenario).is_err());
    }
}
