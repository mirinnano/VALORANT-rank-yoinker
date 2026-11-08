//! 記録した応答を返す [`RiotSource`]。VALORANT なしで動作を確認するために使う。

use crate::error::{Error, Result};
use crate::presence::Presence;
use crate::source::{GameCatalog, RiotSource};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Scenario {
    pub own: String,
    /// `presences()` の呼び出しごとに次へ進み、末尾の次は先頭へ戻る。
    pub timeline: Vec<Vec<Value>>,
    pub pregame: Option<Value>,
    pub coregame: Option<Value>,
    pub names: HashMap<String, String>,
    pub mmr: HashMap<String, Value>,
    pub updates: HashMap<String, Value>,
    pub history: Option<Value>,
    /// 履歴の取得を失敗させる。再試行の確認に使う。
    #[serde(skip)]
    pub fail_history: bool,
    /// 全モードの試合一覧。`None` なら `history` のコンペティティブから作る。
    pub match_list: Option<Value>,
    /// 試合一覧の取得を失敗させる。
    #[serde(skip)]
    pub fail_match_list: bool,
    pub matches: HashMap<String, Value>,
    pub content: Value,
    pub agents: HashMap<String, String>,
    pub maps: HashMap<String, String>,
    pub weapons: HashMap<String, String>,
    pub party: Option<Value>,
    pub storefront: Option<Value>,
    pub wallet: Option<Value>,
}

pub struct ReplaySource {
    scenario: Scenario,
    cursor: AtomicUsize,
    requested_names: Mutex<HashSet<String>>,
    pregame_calls: AtomicUsize,
    coregame_calls: AtomicUsize,
    history_calls: AtomicUsize,
    party_calls: AtomicUsize,
    live: bool,
    mmr_gate: Option<Arc<Semaphore>>,
}

impl ReplaySource {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            cursor: AtomicUsize::new(0),
            requested_names: Mutex::default(),
            pregame_calls: AtomicUsize::new(0),
            coregame_calls: AtomicUsize::new(0),
            history_calls: AtomicUsize::new(0),
            party_calls: AtomicUsize::new(0),
            live: false,
            mmr_gate: None,
        }
    }

    pub fn pregame_calls(&self) -> usize {
        self.pregame_calls.load(Ordering::Relaxed)
    }

    /// mmr の応答を `gate` の許可が出るまで止める。途中経過の確認に使う。
    pub fn with_mmr_gate(mut self, gate: Arc<Semaphore>) -> Self {
        self.mmr_gate = Some(gate);
        self
    }

    /// WebSocket に接続している状態として振る舞う。
    pub fn with_live_events(mut self) -> Self {
        self.live = true;
        self
    }

    pub fn party_calls(&self) -> usize {
        self.party_calls.load(Ordering::Relaxed)
    }

    pub fn history_calls(&self) -> usize {
        self.history_calls.load(Ordering::Relaxed)
    }

    pub fn coregame_calls(&self) -> usize {
        self.coregame_calls.load(Ordering::Relaxed)
    }

    /// name-service へ問い合わせた puuid の一覧。
    pub fn requested_names(&self) -> HashSet<String> {
        self.requested_names.lock().expect("name log poisoned").clone()
    }
}

#[async_trait]
impl RiotSource for ReplaySource {
    async fn connect(&self) -> Result<String> {
        Ok(self.scenario.own.clone())
    }

    async fn own_puuid(&self) -> Result<String> {
        Ok(self.scenario.own.clone())
    }

    async fn presences(&self) -> Result<Vec<Presence>> {
        let timeline = &self.scenario.timeline;
        if timeline.is_empty() {
            return Err(Error::ClientNotReady);
        }
        let index = self.cursor.fetch_add(1, Ordering::Relaxed) % timeline.len();
        timeline[index]
            .iter()
            .map(|value| serde_json::from_value(value.clone()).map_err(Error::from))
            .collect()
    }

    async fn pregame(&self, _puuid: &str) -> Result<Option<Value>> {
        self.pregame_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.scenario.pregame.clone())
    }

    async fn coregame(&self, _puuid: &str) -> Result<Option<Value>> {
        self.coregame_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.scenario.coregame.clone())
    }

    async fn names(&self, puuids: &[String]) -> Result<HashMap<String, String>> {
        self.requested_names.lock().expect("name log poisoned").extend(puuids.iter().cloned());
        Ok(puuids
            .iter()
            .filter_map(|id| Some((id.clone(), self.scenario.names.get(id)?.clone())))
            .collect())
    }

    async fn mmr(&self, puuid: &str) -> Result<Option<Value>> {
        if let Some(gate) = &self.mmr_gate {
            gate.acquire().await.expect("gate is never closed").forget();
        }
        Ok(self.scenario.mmr.get(puuid).cloned())
    }

    async fn competitive_updates(&self, puuid: &str) -> Result<Option<Value>> {
        Ok(self.scenario.updates.get(puuid).cloned())
    }

    async fn rank_history(&self, _puuid: &str, start: u32, end: u32) -> Result<Option<Value>> {
        self.history_calls.fetch_add(1, Ordering::Relaxed);
        if self.scenario.fail_history {
            return Err(Error::Status { service: "pd", status: 503 });
        }
        // 実物と同じように、指定された範囲だけを返す。
        Ok(self.scenario.history.as_ref().map(|history| {
            let matches: Vec<Value> = history
                .get("Matches")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .skip(start as usize)
                .take(end.saturating_sub(start) as usize)
                .cloned()
                .collect();
            serde_json::json!({ "Matches": matches })
        }))
    }

    fn events_connected(&self) -> Option<bool> {
        self.live.then_some(true)
    }

    async fn party_player(&self, puuid: &str) -> Result<Option<Value>> {
        let id = self.scenario.party.as_ref().and_then(|party| party.get("ID")).cloned();
        Ok(id.map(|id| serde_json::json!({ "Subject": puuid, "CurrentPartyID": id })))
    }

    async fn party(&self, _party_id: &str) -> Result<Option<Value>> {
        self.party_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.scenario.party.clone())
    }

    async fn set_party_invite(&self, _party_id: &str, enabled: bool) -> Result<Option<Value>> {
        let code = if enabled { "DEMO42" } else { "" };
        Ok(self.scenario.party.clone().map(|mut party| {
            party["InviteCode"] = code.into();
            party
        }))
    }

    async fn storefront(&self, _puuid: &str) -> Result<Option<Value>> {
        Ok(self.scenario.storefront.clone())
    }

    async fn wallet(&self, _puuid: &str) -> Result<Option<Value>> {
        Ok(self.scenario.wallet.clone())
    }

    async fn match_history(&self, _puuid: &str, start: u32, end: u32) -> Result<Option<Value>> {
        if self.scenario.fail_match_list {
            return Err(Error::Status { service: "pd", status: 503 });
        }
        let entries: Vec<Value> = match &self.scenario.match_list {
            Some(list) => list.get("History").and_then(Value::as_array).cloned().unwrap_or_default(),
            None => self
                .scenario
                .history
                .as_ref()
                .and_then(|history| history.get("Matches"))
                .and_then(Value::as_array)
                .map(|matches| {
                    matches
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "MatchID": m.get("MatchID"),
                                "GameStartTime": m.get("MatchStartTime"),
                                "QueueID": "competitive",
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let total = entries.len();
        let page: Vec<Value> =
            entries.into_iter().skip(start as usize).take(end.saturating_sub(start) as usize).collect();
        Ok(Some(serde_json::json!({ "BeginIndex": start, "EndIndex": end, "Total": total, "History": page })))
    }

    async fn match_details(&self, match_id: &str) -> Result<Option<Value>> {
        Ok(self.scenario.matches.get(match_id).cloned())
    }

    async fn content(&self) -> Result<Value> {
        Ok(self.scenario.content.clone())
    }

    async fn catalog(&self) -> Result<GameCatalog> {
        Ok(GameCatalog {
            agents: self.scenario.agents.clone(),
            maps: self.scenario.maps.clone(),
            weapons: self.scenario.weapons.clone(),
        })
    }
}
