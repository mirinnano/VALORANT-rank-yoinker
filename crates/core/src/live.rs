//! Riot Client と Riot のサーバーへの接続。
//!
//! 証明書検証を無効にするのは、自己署名証明書を使う 127.0.0.1 の
//! Riot Client 向けクライアントだけにする。リモート向けは常に検証する。

use crate::error::{Error, Result};
use crate::local::{parse_client_log, read_lossy, LocalPaths, Lockfile};
use crate::presence::{self, LiveEvent, Presence};
use crate::retry::{with_policy, Reply};
use crate::source::{GameCatalog, LiveSignals, RiotSource};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{header, redirect, Client, Method, RequestBuilder};
use futures_util::{SinkExt, StreamExt};
use native_tls::TlsConnector;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OnceCell, RwLock, Semaphore};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_tls_with_config, Connector};

const CLIENT_PLATFORM: &str = "ew0KCSJwbGF0Zm9ybVR5cGUiOiAiUEMiLA0KCSJwbGF0Zm9ybU9TIjog\
IldpbmRvd3MiLA0KCSJwbGF0Zm9ybU9TVmVyc2lvbiI6ICIxMC4wLjE5MDQyLjEuMjU2LjY0Yml0IiwNCgkicGxhdGZvcm1DaGlwc2V0IjogIlVua25vd24iDQp9";
const USER_AGENT: &str = "ShooterGame/13 Windows/10.0.19043.1.256.64bit";
const VALORANT_API: &str = "https://valorant-api.com/v1";

const MAX_CONCURRENT: usize = 4;
const MIN_INTERVAL: Duration = Duration::from_millis(150);
const RATE_LIMIT_HOLD: Duration = Duration::from_secs(5);
const EVENT_RECONNECT: Duration = Duration::from_secs(5);
const EVENT_SUBSCRIPTIONS: [&str; 2] = [
    r#"[5, "OnJsonApiEvent_chat_v4_presences"]"#,
    r#"[5, "OnJsonApiEvent_riot-messaging-service_v1_message"]"#,
];

/// WebSocket のエラー。URL を含まないよう文字列にしない。
#[derive(Debug, thiserror::Error)]
enum EventError {
    #[error("WebSocket の接続に失敗: {0}")]
    Socket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("TLS の設定に失敗: {0}")]
    Tls(#[from] native_tls::Error),
    #[error("認証ヘッダーの作成に失敗")]
    Header,
}

#[derive(Clone)]
struct Session {
    lockfile: Lockfile,
    puuid: String,
    access_token: String,
    entitlement: String,
    client_version: String,
    pd_url: String,
    glz_url: String,
    shared_url: String,
}

#[derive(Deserialize)]
struct Entitlements {
    subject: String,
    #[serde(rename = "accessToken")]
    access_token: String,
    token: String,
}

pub struct LiveSource {
    local: Client,
    remote: Client,
    paths: LocalPaths,
    session: RwLock<Option<Arc<Session>>>,
    permits: Semaphore,
    next_slot: Mutex<Instant>,
    catalog: OnceCell<GameCatalog>,
    events_connected: AtomicBool,
    events_received: AtomicU64,
}

impl LiveSource {
    pub fn new(paths: LocalPaths) -> Result<Self> {
        let local = Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .redirect(redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()?;
        let remote = Client::builder().timeout(Duration::from_secs(15)).build()?;
        Ok(Self {
            local,
            remote,
            paths,
            session: RwLock::new(None),
            permits: Semaphore::new(MAX_CONCURRENT),
            next_slot: Mutex::new(Instant::now()),
            catalog: OnceCell::new(),
            events_connected: AtomicBool::new(false),
            events_received: AtomicU64::new(0),
        })
    }

    fn local_request(&self, lock: &Lockfile, method: Method, path: &str) -> RequestBuilder {
        let auth = STANDARD.encode(format!("riot:{}", lock.password));
        self.local
            .request(method, format!("https://127.0.0.1:{}{path}", lock.port))
            .header(header::AUTHORIZATION, format!("Basic {auth}"))
    }

    async fn establish(&self) -> Result<Arc<Session>> {
        let lockfile = Lockfile::parse(&read_lossy(&self.paths.lockfile)?)?;
        let log = parse_client_log(&read_lossy(&self.paths.client_log)?)
            .ok_or(Error::ClientStep("ログに接続先がまだありません"))?;
        let response = self
            .local_request(&lockfile, Method::GET, "/entitlements/v1/token")
            .send()
            .await
            .map_err(|_| Error::ClientStep("Riot Client に接続できません"))?;
        if !response.status().is_success() {
            return Err(Error::ClientStep("トークンを取得できません"));
        }
        let entitlements: Entitlements =
            response.json().await.map_err(|_| Error::ClientStep("トークンの形式が不明です"))?;

        let session = Arc::new(Session {
            puuid: entitlements.subject,
            access_token: entitlements.access_token,
            entitlement: entitlements.token,
            client_version: log.client_version,
            pd_url: format!("https://pd.{}.a.pvp.net", log.region),
            glz_url: format!("https://glz-{}.{}.a.pvp.net", log.glz_region, log.glz_shard),
            shared_url: format!("https://shared.{}.a.pvp.net", log.region),
            lockfile,
        });
        *self.session.write().await = Some(session.clone());
        Ok(session)
    }

    async fn current(&self) -> Result<Arc<Session>> {
        if let Some(session) = self.session.read().await.clone() {
            return Ok(session);
        }
        self.establish().await
    }

    /// リクエストの間隔を空け、バースト送信を防ぐ。
    async fn pace(&self) {
        let wait_until = {
            let mut next = self.next_slot.lock().await;
            let slot = (*next).max(Instant::now());
            *next = slot + MIN_INTERVAL;
            slot
        };
        tokio::time::sleep_until(wait_until).await;
    }

    /// Riot Client の WebSocket へ接続し、自分の状態が変わるたびに起こす。
    async fn read_events(&self, session: &Session, signals: &LiveSignals) -> std::result::Result<(), EventError> {
        let auth = STANDARD.encode(format!("riot:{}", session.lockfile.password));
        let mut request = format!("wss://127.0.0.1:{}", session.lockfile.port).into_client_request()?;
        let value = format!("Basic {auth}").parse().map_err(|_| EventError::Header)?;
        request.headers_mut().insert(header::AUTHORIZATION, value);

        // 自己署名証明書を受け入れるのは、この 127.0.0.1 への接続だけ。
        let tls = TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()?;
        let (mut socket, _) =
            connect_async_tls_with_config(request, None, false, Some(Connector::NativeTls(tls))).await?;
        for subscription in EVENT_SUBSCRIPTIONS {
            socket.send(Message::text(subscription)).await?;
        }

        self.events_connected.store(true, Ordering::Relaxed);
        signals.wake.notify_one();
        tracing::info!("WebSocket に接続");
        let mut last = None;
        while let Some(message) = socket.next().await {
            let Message::Text(text) = message? else { continue };
            let Some(event) = presence::parse_live_event(text.as_str(), &session.puuid) else { continue };
            if self.events_received.fetch_add(1, Ordering::Relaxed) == 0 {
                tracing::info!("WebSocket の最初のイベントを受信");
            }
            match event {
                // 状態だけでなく、スコアや人数の変化でも起こす。
                LiveEvent::OwnPresence(own) => {
                    if last.as_ref() != Some(&own) {
                        last = Some(own);
                        signals.wake.notify_one();
                    }
                }
                LiveEvent::PartyChanged => {
                    signals.party_changed.store(true, Ordering::Relaxed);
                    signals.wake.notify_one();
                }
                LiveEvent::PregameChanged => {
                    signals.pregame_changed.store(true, Ordering::Relaxed);
                    signals.wake.notify_one();
                }
            }
        }
        Ok(())
    }

    /// レート制限を受けたら、全体の送信を止める。
    async fn hold_requests(&self, delay: Duration) {
        let mut next = self.next_slot.lock().await;
        *next = (*next).max(Instant::now() + delay);
    }

    async fn remote_call(
        &self,
        service: &'static str,
        method: Method,
        url: impl Fn(&Session) -> String,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        let text = with_policy(
            service,
            || async {
                // 枠は送信の間だけ持つ。待機中に持つと、ほかの通信まで止まる。
                let _permit = self.permits.acquire().await.expect("semaphore is never closed");
                self.pace().await;
                let session = self.current().await?;
                let mut request = self
                    .remote
                    .request(method.clone(), url(&session))
                    .bearer_auth(&session.access_token)
                    .header("X-Riot-Entitlements-JWT", &session.entitlement)
                    .header("X-Riot-ClientPlatform", CLIENT_PLATFORM)
                    .header("X-Riot-ClientVersion", &session.client_version)
                    .header(header::USER_AGENT, USER_AGENT);
                if let Some(body) = body {
                    request = request.json(body);
                }
                let reply = read_reply(request.send().await?).await?;
                if reply.status == 429 {
                    self.hold_requests(reply.retry_after.unwrap_or(RATE_LIMIT_HOLD)).await;
                }
                Ok(reply)
            },
            || async { self.establish().await.map(drop) },
        )
        .await?;
        text.map(|text| serde_json::from_str(&text).map_err(Error::from)).transpose()
    }

    async fn glz(&self, path: String) -> Result<Option<Value>> {
        self.remote_call("glz", Method::GET, |s| format!("{}{path}", s.glz_url), None).await
    }

    async fn pd(&self, path: String) -> Result<Option<Value>> {
        self.remote_call("pd", Method::GET, |s| format!("{}{path}", s.pd_url), None).await
    }

    /// `MatchID` を引いてから試合本体を取得する。
    async fn active_match(&self, kind: &str, puuid: &str) -> Result<Option<Value>> {
        let Some(player) = self.glz(format!("/{kind}/v1/players/{puuid}")).await? else {
            return Ok(None);
        };
        let Some(id) = player.get("MatchID").and_then(Value::as_str) else {
            return Ok(None);
        };
        self.glz(format!("/{kind}/v1/matches/{id}")).await
    }

    async fn public_data(&self, path: &str) -> Result<Vec<Value>> {
        let value: Value = self
            .remote
            .get(format!("{VALORANT_API}{path}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        match value.get("data") {
            Some(Value::Array(items)) => Ok(items.clone()),
            _ => Err(Error::Decode(format!("valorant-api{path}"))),
        }
    }
}

async fn read_reply(response: reqwest::Response) -> Result<Reply> {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs);
    Ok(Reply { status, retry_after, body: response.text().await? })
}

fn lookup_table(items: &[Value], key: &str) -> HashMap<String, String> {
    items
        .iter()
        .filter_map(|item| {
            let key = item.get(key)?.as_str()?.to_lowercase();
            let name = item.get("displayName")?.as_str()?.to_owned();
            Some((key, name))
        })
        .collect()
}

#[async_trait]
impl RiotSource for LiveSource {
    async fn connect(&self) -> Result<String> {
        Ok(self.establish().await?.puuid.clone())
    }

    async fn own_puuid(&self) -> Result<String> {
        Ok(self.current().await?.puuid.clone())
    }

    async fn presences(&self) -> Result<Vec<Presence>> {
        #[derive(Deserialize)]
        struct Reply {
            presences: Vec<Presence>,
        }
        let session = self.current().await?;
        let response = self
            .local_request(&session.lockfile, Method::GET, "/chat/v4/presences")
            .send()
            .await
            .map_err(|_| Error::ClientNotReady)?;
        if !response.status().is_success() {
            return Err(Error::ClientNotReady);
        }
        Ok(response.json::<Reply>().await.map_err(|_| Error::ClientNotReady)?.presences)
    }

    async fn pregame(&self, puuid: &str) -> Result<Option<Value>> {
        self.active_match("pregame", puuid).await
    }

    async fn coregame(&self, puuid: &str) -> Result<Option<Value>> {
        self.active_match("core-game", puuid).await
    }

    async fn names(&self, puuids: &[String]) -> Result<HashMap<String, String>> {
        if puuids.is_empty() {
            return Ok(HashMap::new());
        }
        let body = serde_json::to_value(puuids)?;
        let reply = self
            .remote_call("pd", Method::PUT, |s| format!("{}/name-service/v2/players", s.pd_url), Some(&body))
            .await?
            .ok_or_else(|| Error::Decode("name-service".into()))?;
        let entries = reply.as_array().ok_or_else(|| Error::Decode("name-service".into()))?;
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let puuid = entry.get("Subject")?.as_str()?.to_owned();
                let name = entry.get("GameName")?.as_str()?;
                let tag = entry.get("TagLine")?.as_str()?;
                Some((puuid, format!("{name}#{tag}")))
            })
            .collect())
    }

    async fn mmr(&self, puuid: &str) -> Result<Option<Value>> {
        self.pd(format!("/mmr/v1/players/{puuid}")).await
    }

    async fn competitive_updates(&self, puuid: &str) -> Result<Option<Value>> {
        self.pd(format!(
            "/mmr/v1/players/{puuid}/competitiveupdates?startIndex=0&endIndex=5&queue=competitive"
        ))
        .await
    }

    async fn rank_history(&self, puuid: &str, start: u32, end: u32) -> Result<Option<Value>> {
        self.pd(format!(
            "/mmr/v1/players/{puuid}/competitiveupdates?startIndex={start}&endIndex={end}&queue=competitive"
        ))
        .await
    }

    async fn match_details(&self, match_id: &str) -> Result<Option<Value>> {
        self.pd(format!("/match-details/v1/matches/{match_id}")).await
    }

    async fn party_player(&self, puuid: &str) -> Result<Option<Value>> {
        self.glz(format!("/parties/v1/players/{puuid}")).await
    }

    async fn party(&self, party_id: &str) -> Result<Option<Value>> {
        self.glz(format!("/parties/v1/parties/{party_id}")).await
    }

    async fn set_party_invite(&self, party_id: &str, enabled: bool) -> Result<Option<Value>> {
        let method = if enabled { Method::POST } else { Method::DELETE };
        let path = format!("/parties/v1/parties/{party_id}/invitecode");
        self.remote_call("glz", method, |s| format!("{}{path}", s.glz_url), None).await
    }

    async fn storefront(&self, puuid: &str) -> Result<Option<Value>> {
        let path = format!("/store/v3/storefront/{puuid}");
        let body = serde_json::json!({});
        self.remote_call("pd", Method::POST, |s| format!("{}{path}", s.pd_url), Some(&body)).await
    }

    async fn wallet(&self, puuid: &str) -> Result<Option<Value>> {
        self.pd(format!("/store/v1/wallet/{puuid}")).await
    }

    async fn content(&self) -> Result<Value> {
        self.remote_call(
            "shared",
            Method::GET,
            |s| format!("{}/content-service/v3/content", s.shared_url),
            None,
        )
        .await?
        .ok_or_else(|| Error::Decode("content-service".into()))
    }

    async fn watch_events(&self, signals: Arc<LiveSignals>) {
        loop {
            // 再接続のたびにセッションを読み直す。Riot Client が再起動すると接続先が変わる。
            match self.establish().await {
                Ok(session) => {
                    match self.read_events(&session, &signals).await {
                        Ok(()) => tracing::warn!("WebSocket が閉じられました"),
                        Err(error) => tracing::warn!(error = %error, "WebSocket が切断"),
                    }
                    self.events_connected.store(false, Ordering::Relaxed);
                    // 待機中のエンジンを起こし、定期更新へ切り替えさせる。
                    signals.wake.notify_one();
                }
                Err(error) => tracing::debug!(%error, "WebSocket の接続を待っています"),
            }
            tokio::time::sleep(EVENT_RECONNECT).await;
        }
    }

    fn events_connected(&self) -> Option<bool> {
        Some(self.events_connected.load(Ordering::Relaxed))
    }

    fn events_received(&self) -> Option<u64> {
        Some(self.events_received.load(Ordering::Relaxed))
    }

    async fn catalog(&self) -> Result<GameCatalog> {
        // 起動中に変わらないので、一度だけ取得する。
        self.catalog
            .get_or_try_init(|| async {
                let agents = self.public_data("/agents?isPlayableCharacter=true").await?;
                let maps = self.public_data("/maps").await?;
                let weapons = self.public_data("/weapons").await?;
                Ok(GameCatalog {
                    agents: lookup_table(&agents, "uuid"),
                    maps: lookup_table(&maps, "mapUrl"),
                    weapons: lookup_table(&weapons, "uuid"),
                })
            })
            .await
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn client_platform_constant_is_valid_base64_json() {
        let bytes = STANDARD.decode(CLIENT_PLATFORM).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["platformType"], "PC");
    }

    #[test]
    fn lookup_table_lowercases_keys_and_skips_broken_items() {
        let items = [json!({"uuid": "ABC", "displayName": "Jett"}), json!({"uuid": "x"})];
        let table = lookup_table(&items, "uuid");
        assert_eq!(table.len(), 1);
        assert_eq!(table["abc"], "Jett");
    }
}
