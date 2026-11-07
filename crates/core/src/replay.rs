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
    pub matches: HashMap<String, Value>,
    pub content: Value,
    pub agents: HashMap<String, String>,
    pub maps: HashMap<String, String>,
}

pub struct ReplaySource {
    scenario: Scenario,
    cursor: AtomicUsize,
    requested_names: Mutex<HashSet<String>>,
    pregame_calls: AtomicUsize,
    coregame_calls: AtomicUsize,
    history_calls: AtomicUsize,
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

    async fn rank_history(&self, _puuid: &str, _count: u32) -> Result<Option<Value>> {
        self.history_calls.fetch_add(1, Ordering::Relaxed);
        if self.scenario.fail_history {
            return Err(Error::Status { service: "pd", status: 503 });
        }
        Ok(self.scenario.history.clone())
    }

    async fn match_details(&self, match_id: &str) -> Result<Option<Value>> {
        Ok(self.scenario.matches.get(match_id).cloned())
    }

    async fn content(&self) -> Result<Value> {
        Ok(self.scenario.content.clone())
    }

    async fn catalog(&self) -> Result<GameCatalog> {
        Ok(GameCatalog { agents: self.scenario.agents.clone(), maps: self.scenario.maps.clone() })
    }
}
