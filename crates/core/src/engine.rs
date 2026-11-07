//! 状態の監視とスナップショットの生成。

use crate::analytics;
use crate::content::Seasons;
use crate::error::{Error, Result};
use crate::model::{AppEvent, GameState, MatchSnapshot, PickBoard, PickStat};
use crate::presence::{self, Presence, PrivatePresence};
use crate::profile::{self, Profile};
use crate::rank;
use crate::roster::{self, BuildContext, PlayerExtra, RawMatch};
use crate::source::{GameCatalog, RiotSource};
use crate::stats;
use crate::store::Store;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, OnceCell};
use tokio::task::JoinSet;
use tokio::time::Instant;

const TICK: Duration = Duration::from_secs(3);
const RECONNECT: Duration = Duration::from_secs(5);
/// エージェント選択は変化を拾うため、数ティックごとに取り直す。
const PREGAME_REFRESH_TICKS: u32 = 2;
const HISTORY_MATCHES: u32 = 20;
/// ピックの観測に含める、チーム平均からのランクの幅。1 ランクは 3 段。
const PICK_TIER_RANGE: u32 = 3;
const PICK_MIN_MATCHES: u32 = 5;
/// 取得に失敗した分を取り直す間隔。短いとレート制限を長引かせる。
const INCOMPLETE_RETRY_TICKS: u32 = 10;
/// プロフィールの取得に失敗したら、このティック数だけ待って取り直す。
const PROFILE_RETRY_TICKS: u32 = 10;

pub fn mode_name(queue: Option<&str>) -> Option<String> {
    let name = match queue? {
        "newmap" => "New Map",
        "competitive" => "Competitive",
        "unrated" => "Unrated",
        "swiftplay" => "Swiftplay",
        "spikerush" => "Spike Rush",
        "deathmatch" => "Deathmatch",
        "ggteam" => "Escalation",
        "onefa" => "Replication",
        "hurm" => "Team Deathmatch",
        "custom" | "" => "Custom",
        "snowball" => "Snowball Fight",
        "valaram" => "All Random One Site",
        "dodgeball" => "Knockout",
        "abilitydraftarena" => "Gauntlet: Glitched",
        "skirmish2v2" => "Skirmish: 2v2",
        other => other,
    };
    Some(name.to_owned())
}

fn snapshot_mode(own: &PrivatePresence) -> Option<String> {
    if own.custom_game {
        return Some("Custom Game".to_owned());
    }
    mode_name(own.queue_id.as_deref())
}

type DetailsCache = Arc<Mutex<HashMap<String, Arc<OnceCell<Option<Value>>>>>>;

struct Connection {
    puuid: String,
    seasons: Arc<Seasons>,
    catalog: GameCatalog,
}

/// 取得結果と、再試行が必要かどうか。
struct FetchedExtra {
    puuid: String,
    extra: PlayerExtra,
    complete: bool,
}

async fn fetch_extra(
    source: Arc<dyn RiotSource>,
    seasons: Arc<Seasons>,
    details: DetailsCache,
    puuid: String,
) -> FetchedExtra {
    let mut complete = true;
    let mmr = source.mmr(&puuid).await.unwrap_or_else(|error| {
        tracing::warn!(%error, "mmr の取得に失敗");
        complete = false;
        None
    });

    let current = seasons.current_id();
    let rank_now = rank::season_rank(mmr.as_ref(), current);
    let previous = rank::season_rank(mmr.as_ref(), seasons.previous_id());
    let peak = rank::peak_rank(mmr.as_ref(), &rank_now, current);
    let peak_act = peak.season.as_deref().and_then(|id| seasons.act_label(id));

    let updates = source.competitive_updates(&puuid).await.unwrap_or_else(|error| {
        tracing::warn!(%error, "competitive updates の取得に失敗");
        complete = false;
        None
    });

    let mut recent = stats::RecentStats::default();
    if let Some(summary) = updates.as_ref().and_then(stats::latest_match) {
        let match_id = summary.get("MatchID").and_then(Value::as_str).map(str::to_owned);
        let mut match_data = None;
        if let Some(id) = match_id {
            let cell = {
                let mut cache = details.lock().expect("details cache poisoned");
                cache.entry(id.clone()).or_default().clone()
            };
            let result = cell
                .get_or_try_init(|| async { source.match_details(&id).await })
                .await;
            match result {
                Ok(value) => match_data = value.clone(),
                Err(error) => {
                    tracing::warn!(%error, "match details の取得に失敗");
                    complete = false;
                }
            }
        }
        recent = stats::from_match(&puuid, summary, match_data.as_ref());
    }
    recent.form = updates.as_ref().map(stats::recent_form).unwrap_or_default();

    let extra = PlayerExtra { rank: rank_now, previous, peak, peak_act, stats: recent };
    FetchedExtra { puuid, extra, complete }
}

/// 取得の途中でも変わらない、スナップショットの材料。
struct MatchView<'a> {
    puuid: &'a str,
    party: &'a HashSet<String>,
    names: &'a HashMap<String, String>,
    encounters: &'a HashMap<String, u32>,
    presences: &'a [Presence],
    mode: Option<String>,
    score: Option<(u32, u32)>,
}

pub struct Engine {
    source: Arc<dyn RiotSource>,
    store: Option<Store>,
    connection: Option<Connection>,
    extras: HashMap<String, PlayerExtra>,
    /// 取得に失敗した分があり、次の更新で取り直すプレイヤー。
    incomplete: HashSet<String>,
    /// 表示順が変わっても UI が同じプレイヤーを追える連番。
    player_keys: HashMap<String, u32>,
    details: DetailsCache,
    active_match: Option<String>,
    current: Option<MatchSnapshot>,
    pregame_ticks: u32,
    profile: Option<Profile>,
    profile_wait: u32,
    skipped_metrics: HashSet<String>,
    was_in_match: bool,
    playing_match: Option<String>,
    finished_match: Option<String>,
    progress: Option<watch::Sender<AppEvent>>,
}

fn system_now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Engine {
    pub fn new(source: Arc<dyn RiotSource>, store: Option<Store>) -> Self {
        Self {
            source,
            store,
            connection: None,
            extras: HashMap::new(),
            incomplete: HashSet::new(),
            player_keys: HashMap::new(),
            details: DetailsCache::default(),
            active_match: None,
            current: None,
            pregame_ticks: 0,
            profile: None,
            profile_wait: 0,
            skipped_metrics: HashSet::new(),
            was_in_match: false,
            playing_match: None,
            finished_match: None,
            progress: None,
        }
    }

    /// 取得の途中経過を流す先を設定する。
    pub fn with_progress(mut self, events: watch::Sender<AppEvent>) -> Self {
        self.progress = Some(events);
        self
    }

    fn publish(&self, mut snapshot: MatchSnapshot) {
        if let Some(events) = &self.progress {
            snapshot.profile = self.profile.clone();
            snapshot.finished_match = self.finished_match.clone();
            events.send_replace(AppEvent::Snapshot(snapshot));
        }
    }

    async fn ensure_connected(&mut self) -> Result<()> {
        if self.connection.is_some() {
            return Ok(());
        }
        let started = Instant::now();
        let puuid = self.source.connect().await?;
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "Riot Client に接続");
        let content = self.source.content().await?;
        let seasons = Arc::new(Seasons::from_content(&content)?);
        let catalog = self.source.catalog().await?;
        tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "接続の準備が完了");
        self.connection = Some(Connection { puuid, seasons, catalog });
        Ok(())
    }

    fn reset_match(&mut self) {
        self.extras.clear();
        self.incomplete.clear();
        self.player_keys.clear();
        self.details.lock().expect("details cache poisoned").clear();
        self.active_match = None;
        self.current = None;
        self.pregame_ticks = 0;
    }

    /// 接続を破棄する。次の [`Engine::step`] で接続し直す。
    pub fn disconnect(&mut self) {
        self.connection = None;
        self.profile = None;
        self.profile_wait = 0;
        self.reset_match();
    }

    /// 1 回分の更新を行い、現在のスナップショットを返す。
    ///
    /// プロフィールは状態を問わず読む。試合の途中で起動しても、ホームが空にならないように。
    pub async fn step(&mut self) -> Result<MatchSnapshot> {
        let mut snapshot = self.update_match().await?;
        if let Some(puuid) = self.connection.as_ref().map(|c| c.puuid.clone()) {
            // プロフィールの取得を待たず、先に現在の状態を見せる。
            if self.profile.is_none() {
                self.publish(snapshot.clone());
            }
            self.load_profile(&puuid).await;
            if snapshot.state != GameState::Pregame {
                self.sync_metrics(&puuid).await;
            }
            snapshot.profile = self.profile.clone();
        }
        snapshot.finished_match = self.finished_match.clone();
        Ok(snapshot)
    }

    async fn update_match(&mut self) -> Result<MatchSnapshot> {
        self.ensure_connected().await?;
        let puuid = self.connection.as_ref().expect("connected above").puuid.clone();

        let presences = self.source.presences().await?;
        let own = presence::find_own(&presences, &puuid).ok_or(Error::ClientNotReady)?;
        let state = own.state.ok_or(Error::ClientNotReady)?;
        let mode = snapshot_mode(&own);

        let raw = match state {
            GameState::Menus => {
                self.reset_match();
                if self.was_in_match {
                    self.profile = None;
                    self.was_in_match = false;
                    self.profile_wait = 0;
                    self.finished_match = self.playing_match.take().or(self.finished_match.take());
                }
                return Ok(MatchSnapshot::empty(state, mode));
            }
            GameState::Pregame => {
                if let Some(snapshot) = self.reusable(state) {
                    self.pregame_ticks += 1;
                    if self.pregame_ticks % PREGAME_REFRESH_TICKS != 0 {
                        return Ok(snapshot);
                    }
                }
                match self.source.pregame(&puuid).await? {
                    Some(value) => roster::parse_pregame(&value)?,
                    None => return Ok(MatchSnapshot::empty(state, mode)),
                }
            }
            GameState::InGame => {
                if let Some(mut snapshot) = self.reusable(state) {
                    snapshot.score = own.score;
                    return Ok(snapshot);
                }
                match self.source.coregame(&puuid).await? {
                    Some(value) => roster::parse_coregame(&value)?,
                    None => return Ok(MatchSnapshot::empty(state, mode)),
                }
            }
        };

        self.was_in_match = true;
        let mut snapshot = self.build(raw, &presences, &puuid, mode, own.score).await?;
        self.save_cached_picks();
        if snapshot.state == GameState::Pregame {
            if let Some(map) = snapshot.map.clone() {
                let tier = snapshot.ally_average.as_ref().map_or(0, |rank| rank.tier);
                let allies: Vec<String> =
                    snapshot.players.iter().filter_map(|player| player.agent.clone()).collect();
                snapshot.pick_board = self.pick_board(&map, tier, &allies);
            }
        }
        if snapshot.state == GameState::InGame {
            self.playing_match = snapshot.match_id.clone();
            if let Some(store) = self.store.as_mut() {
                if let Err(error) = store.record(&snapshot, system_now_ms()) {
                    tracing::warn!(%error, "遭遇履歴の保存に失敗");
                }
            }
        }
        self.current = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// プロフィールを返す。失敗しても試合の表示は止めない。
    async fn load_profile(&mut self, puuid: &str) -> Option<Profile> {
        if self.profile.is_some() {
            return self.profile.clone();
        }
        if self.profile_wait > 0 {
            self.profile_wait -= 1;
            return None;
        }
        let connection = self.connection.as_ref()?;
        let seasons = connection.seasons.clone();
        let own = [puuid.to_owned()];
        let (mmr, history, names) = tokio::join!(
            self.source.mmr(puuid),
            self.source.rank_history(puuid, HISTORY_MATCHES),
            self.source.names(&own),
        );
        let (mmr, history) = match (mmr, history) {
            (Ok(mmr), Ok(history)) => (mmr, history),
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!(%error, "プロフィールの取得に失敗");
                self.profile_wait = PROFILE_RETRY_TICKS;
                return None;
            }
        };
        let name = names.ok().and_then(|mut names| names.remove(puuid));
        let built = profile::build(
            name,
            mmr.as_ref(),
            history.as_ref(),
            &seasons,
            &connection.catalog,
            system_now_ms(),
        );
        self.profile = Some(built.clone());
        Some(built)
    }

    fn save_picks(&mut self, match_id: &str, details: &Value) {
        let Some(connection) = &self.connection else { return };
        let Some((map, picks)) = analytics::observed_picks(details, &connection.catalog) else { return };
        if let Some(store) = &mut self.store {
            if let Err(error) = store.save_picks(match_id, &map, &picks) {
                tracing::warn!(%error, "ピックの保存に失敗");
            }
        }
    }

    /// 取得済みの試合詳細からピックを保存する。味方の直近の試合も観測に使える。
    fn save_cached_picks(&mut self) {
        let cached: Vec<(String, Value)> = {
            let cache = self.details.lock().expect("details cache poisoned");
            cache
                .iter()
                .filter_map(|(id, cell)| Some((id.clone(), cell.get()?.clone()?)))
                .collect()
        };
        for (id, details) in cached {
            self.save_picks(&id, &details);
        }
    }

    /// このマップの観測データを作る。ランク帯の観測が少なければ全ランクを使う。
    fn pick_board(&self, map: &str, average_tier: u32, allies: &[String]) -> Option<PickBoard> {
        let store = self.store.as_ref()?;
        let low = average_tier.saturating_sub(PICK_TIER_RANGE);
        let high = average_tier + PICK_TIER_RANGE;
        let (mut matches, mut counts) = store.pick_counts(map, low, high).ok()?;
        let all_ranks = matches < PICK_MIN_MATCHES;
        if all_ranks {
            (matches, counts) = store.pick_counts(map, 0, u32::MAX).ok()?;
        }
        let (pair_low, pair_high) = if all_ranks { (0, u32::MAX) } else { (low, high) };
        let pairs = store.pair_counts(map, allies, pair_low, pair_high).unwrap_or_default();
        let own = store.all_metrics().unwrap_or_default();
        let on_map: Vec<_> = own.into_iter().filter(|game| game.map.as_deref() == Some(map)).collect();
        let own_stats = analytics::summarize(&on_map).agents;

        let mut agents: Vec<PickStat> = counts
            .into_iter()
            .map(|count| {
                let own = own_stats.iter().find(|entry| entry.name == count.agent);
                PickStat {
                    pick_percent: (count.picks * 100).checked_div(matches).unwrap_or(0).min(100),
                    win_percent: (count.wins * 100).checked_div(count.picks),
                    picks: count.picks,
                    own_games: own.map_or(0, |entry| entry.games),
                    own_wins: own.map_or(0, |entry| entry.wins),
                    own_acs: own.and_then(|entry| entry.acs),
                    agent: count.agent,
                }
            })
            .collect();
        for entry in &own_stats {
            if !agents.iter().any(|agent| agent.agent == entry.name) {
                agents.push(PickStat {
                    agent: entry.name.clone(),
                    own_games: entry.games,
                    own_wins: entry.wins,
                    own_acs: entry.acs,
                    ..Default::default()
                });
            }
        }
        Some(PickBoard { map: map.to_owned(), matches, all_ranks, low_tier: low, high_tier: high, agents, pairs })
    }

    /// 自分の試合の指標を 1 試合ずつ保存し、集計を更新する。
    ///
    /// 一度に取得するとレート制限に当たりやすいので、更新ごとに 1 試合だけ取る。
    async fn sync_metrics(&mut self, puuid: &str) {
        let Some(profile) = &self.profile else { return };
        let ids: Vec<String> =
            profile.history.iter().rev().filter_map(|point| point.match_id.clone()).collect();
        let Some(store) = &self.store else { return };
        let missing = ids
            .iter()
            .find(|id| !self.skipped_metrics.contains(*id) && !store.has_metrics(id).unwrap_or(true))
            .cloned();

        if let Some(id) = missing {
            match self.source.match_details(&id).await {
                Ok(Some(details)) => {
                    let connection = self.connection.as_ref().expect("connected before sync");
                    let metrics = analytics::player_metrics(puuid, &id, &details, &connection.catalog);
                    if let (Some(metrics), Some(store)) = (metrics, &self.store) {
                        if let Err(error) = store.save_metrics(&metrics) {
                            tracing::warn!(%error, "指標の保存に失敗");
                        }
                    }
                    self.save_picks(&id, &details);
                }
                Ok(None) => {
                    self.skipped_metrics.insert(id);
                }
                Err(error) => tracing::warn!(%error, "試合詳細の取得に失敗"),
            }
        }

        let Some(store) = &self.store else { return };
        match store.metrics_for(&ids) {
            Ok(games) if !games.is_empty() => {
                if let Some(profile) = &mut self.profile {
                    profile.summary = Some(analytics::summarize(&games));
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "指標の読み込みに失敗"),
        }
    }

    fn reusable(&self, state: GameState) -> Option<MatchSnapshot> {
        self.current.clone().filter(|snapshot| snapshot.state == state)
    }

    async fn build(
        &mut self,
        mut raw: RawMatch,
        presences: &[Presence],
        puuid: &str,
        mode: Option<String>,
        score: Option<(u32, u32)>,
    ) -> Result<MatchSnapshot> {
        if raw.match_id != self.active_match {
            self.reset_match();
            self.active_match = raw.match_id.clone();
        }

        let party: HashSet<String> = presence::party_members(presences, puuid).into_iter().collect();
        let names = self
            .source
            .names(&roster::names_to_fetch(&raw.players, puuid, &party))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "名前の取得に失敗");
                HashMap::new()
            });

        let ids: Vec<String> = raw.players.iter().map(|p| p.subject.clone()).collect();
        for id in &ids {
            let next = self.player_keys.len() as u32 + 1;
            self.player_keys.entry(id.clone()).or_insert(next);
        }
        let encounters = match &self.store {
            Some(store) => store.counts(&ids, raw.match_id.as_deref()).unwrap_or_default(),
            None => HashMap::new(),
        };

        let view = MatchView { puuid, party: &party, names: &names, encounters: &encounters, presences, mode, score };

        // 画面はすぐに出し、ランクと成績は届いた順に埋める。
        let first = self.render(&mut raw, &view);
        self.publish(first);

        let started = Instant::now();
        let connection = self.connection.as_ref().expect("connected before build");
        let mut tasks = JoinSet::new();
        let retry = self.pregame_ticks % INCOMPLETE_RETRY_TICKS == 0;
        let missing = raw.players.iter().filter(|p| {
            !self.extras.contains_key(&p.subject) || (retry && self.incomplete.contains(&p.subject))
        });
        for player in missing {
            tasks.spawn(fetch_extra(
                self.source.clone(),
                connection.seasons.clone(),
                self.details.clone(),
                player.subject.clone(),
            ));
        }
        while let Some(joined) = tasks.join_next().await {
            let Ok(fetched) = joined else { continue };
            // 失敗時も分かった範囲は表示し、次の更新で取り直す。
            if fetched.complete {
                self.incomplete.remove(&fetched.puuid);
            } else {
                self.incomplete.insert(fetched.puuid.clone());
            }
            self.extras.insert(fetched.puuid, fetched.extra);
            let partial = self.render(&mut raw, &view);
            self.publish(partial);
        }
        tracing::info!(
            players = ids.len(),
            incomplete = self.incomplete.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "プレイヤー情報の取得が完了"
        );

        Ok(self.render(&mut raw, &view))
    }

    /// 現在持っている情報でスナップショットを作る。
    fn render(&self, raw: &mut RawMatch, view: &MatchView<'_>) -> MatchSnapshot {
        let my_team = roster::my_team(&raw.players, view.puuid).map(str::to_owned);
        roster::display_order(&mut raw.players, my_team.as_deref(), &self.extras);
        let ids: Vec<String> = raw.players.iter().map(|p| p.subject.clone()).collect();
        let party_numbers = presence::party_numbers(view.presences, &ids);
        let connection = self.connection.as_ref().expect("connected before build");
        let mut snapshot = roster::build_snapshot(
            raw,
            &BuildContext {
                own: view.puuid,
                party: view.party,
                names: view.names,
                extras: &self.extras,
                encounters: view.encounters,
                party_numbers: &party_numbers,
                keys: &self.player_keys,
                catalog: &connection.catalog,
                mode: view.mode.clone(),
            },
        );
        snapshot.score = view.score;
        snapshot
    }

    /// 終了するまで状態を監視し、変化を `events` へ流す。
    pub async fn run(mut self, events: watch::Sender<AppEvent>) {
        self.progress = Some(events.clone());
        let mut last_state = None;
        loop {
            let delay = match self.step().await {
                Ok(snapshot) => {
                    if last_state != Some(snapshot.state) {
                        tracing::info!(state = ?snapshot.state, players = snapshot.players.len(), "状態が変化");
                        last_state = Some(snapshot.state);
                    }
                    events.send_replace(AppEvent::Snapshot(snapshot));
                    TICK
                }
                Err(error) => {
                    tracing::warn!(%error, "更新に失敗、再接続します");
                    last_state = None;
                    self.disconnect();
                    let reason = match error {
                        Error::ClientNotFound(_) | Error::ClientNotReady | Error::ClientStep(_) => None,
                        other => Some(other.to_string()),
                    };
                    events.send_replace(AppEvent::Connecting { reason });
                    RECONNECT
                }
            };
            tokio::time::sleep(delay).await;
        }
    }
}
