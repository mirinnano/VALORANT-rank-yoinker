//! データ取得の抽象。本番は [`crate::live`]、再生は [`crate::replay`] が実装する。

use crate::error::Result;
use crate::presence::Presence;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Debug, Clone, Default)]
pub struct GameCatalog {
    /// 小文字の agent UUID から表示名。
    pub agents: HashMap<String, String>,
    /// 小文字の mapUrl から表示名。
    pub maps: HashMap<String, String>,
    /// 小文字の武器 UUID から表示名。
    pub weapons: HashMap<String, String>,
}

#[async_trait]
pub trait RiotSource: Send + Sync {
    /// 接続を確立し、自分の puuid を返す。
    async fn connect(&self) -> Result<String>;
    /// 接続済みの自分の puuid を返す。未接続なら接続する。
    async fn own_puuid(&self) -> Result<String>;
    async fn presences(&self) -> Result<Vec<Presence>>;
    /// 進行中のエージェント選択。なければ `None`。
    async fn pregame(&self, puuid: &str) -> Result<Option<Value>>;
    /// 進行中の試合。なければ `None`。
    async fn coregame(&self, puuid: &str) -> Result<Option<Value>>;
    /// puuid から `Name#Tag` を引く。
    async fn names(&self, puuids: &[String]) -> Result<HashMap<String, String>>;
    async fn mmr(&self, puuid: &str) -> Result<Option<Value>>;
    async fn competitive_updates(&self, puuid: &str) -> Result<Option<Value>>;
    /// 直近のコンペティティブの増減。自分のプロフィール用。
    async fn rank_history(&self, puuid: &str, count: u32) -> Result<Option<Value>>;
    async fn match_details(&self, match_id: &str) -> Result<Option<Value>>;
    async fn content(&self) -> Result<Value>;
    async fn catalog(&self) -> Result<GameCatalog>;

    /// 状態の変化を受け取るたびに `wake` を起こす。対応しないデータ源では何もしない。
    async fn watch_events(&self, _wake: Arc<Notify>) {}

    /// イベントの受信が接続中か。対応しないデータ源では `None`。
    fn events_connected(&self) -> Option<bool> {
        None
    }
}
