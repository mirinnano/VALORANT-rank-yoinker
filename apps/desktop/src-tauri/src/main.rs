#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod discord;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, UserAttentionType, WebviewWindow};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::watch;
use vry_core::demo;
use vry_core::diagnostics::{self, Report};
use vry_core::engine::Engine;
use vry_core::images::{self, ImageCatalog};
use vry_core::live::LiveSource;
use vry_core::local::LocalPaths;
use vry_core::model::AppEvent;
use vry_core::replay::{ReplaySource, Scenario};
use vry_core::review;
use vry_core::party;
use vry_core::rich_presence::{self, PresenceOptions};
use vry_core::shop::{self, Personal, ShopCatalog, ShopView, Skin};
use vry_core::source::RiotSource;
use vry_core::store::{Encounter, ReviewNote, Store, TagCount, WishItem};

const APP_EVENT: &str = "app-event";
const RECENT_LIMIT: u32 = 200;
const TAG_WINDOW: u32 = 30;

const WINDOW_MAX: (f64, f64) = (1680.0, 1040.0);
const WINDOW_SCREEN_RATIO: f64 = 0.92;

struct AppState {
    latest: Mutex<AppEvent>,
    database: PathBuf,
    source: Arc<dyn RiotSource>,
    images: tokio::sync::OnceCell<ImageCatalog>,
    discord: Mutex<Sender<discord::Message>>,
    discord_connected: Arc<AtomicBool>,
    discord_options: Mutex<PresenceOptions>,
    shop_catalog: tokio::sync::OnceCell<ShopCatalog>,
}

/// 現在の状態から Discord の表示を作って送る。
fn push_presence(state: &AppState) {
    let event = state.latest.lock().expect("latest event poisoned").clone();
    let options = *state.discord_options.lock().expect("discord options poisoned");
    let view = rich_presence::build(&event, state.images.get(), options);
    let _ = state.discord.lock().expect("discord poisoned").send(discord::Message::Update(view));
}

/// ログをファイルへ書く。リリース版にはコンソールがないため。
fn init_logging(data_dir: &std::path::Path) {
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(data_dir.join("vry.log"))
    else {
        return;
    };
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .init();
}

/// `VRY_REPLAY=demo` なら架空データ、それ以外の値は記録ファイルのパスとして再生する。
fn create_source() -> Result<Arc<dyn RiotSource>, String> {
    match std::env::var("VRY_REPLAY").as_deref() {
        Ok("demo") => return Ok(Arc::new(ReplaySource::new(demo::scenario()))),
        Ok(path) => {
            let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
            let scenario: Scenario = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            return Ok(Arc::new(ReplaySource::new(scenario)));
        }
        Err(_) => {}
    }
    let paths = match LocalPaths::detect() {
        Ok(paths) => paths,
        // Windows 以外でも起動できるよう、存在しない場所を指して接続待ちにする。
        Err(_) => LocalPaths::from_local_app_data(std::path::Path::new("")),
    };
    let source = LiveSource::new(paths).map_err(|error| error.to_string())?;
    Ok(Arc::new(source))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn open_store(state: &AppState) -> Result<Store, String> {
    Store::open(&state.database).map_err(|error| error.to_string())
}

#[tauri::command]
fn latest_event(state: State<'_, AppState>) -> AppEvent {
    state.latest.lock().expect("latest event poisoned").clone()
}

#[tauri::command]
fn recent_encounters(state: State<'_, AppState>) -> Result<Vec<Encounter>, String> {
    open_store(&state)?.recent(RECENT_LIMIT).map_err(|error| error.to_string())
}

/// 試合の振り返りを作る。Riot 側の集計が済んでいなければ `None`。
#[tauri::command]
async fn load_review(
    state: State<'_, AppState>,
    match_id: String,
) -> Result<Option<serde_json::Value>, String> {
    let to_text = |error: vry_core::error::Error| error.to_string();
    if let Some(cached) = open_store(&state)?.cached_review(&match_id).map_err(to_text)? {
        return Ok(Some(cached));
    }

    let source = state.source.clone();
    let own = source.own_puuid().await.map_err(to_text)?;
    let catalog = source.catalog().await.map_err(to_text)?;
    let Some(details) = source.match_details(&match_id).await.map_err(to_text)? else {
        return Ok(None);
    };
    let Some(mut review) = review::analyze(&own, &details, &catalog) else {
        tracing::warn!("試合詳細に自分のデータがありません");
        return Ok(None);
    };
    review.match_id = match_id.clone();
    // ラウンドのない集計は保存しない。保存すると、次回も不完全なまま開いてしまう。
    if !review.rounds.is_empty() {
        let store = open_store(&state)?;
        if let Err(error) = store.cache_review(&match_id, &review) {
            tracing::warn!(%error, "振り返りの保存に失敗");
        }
    }
    serde_json::to_value(&review).map(Some).map_err(|error| error.to_string())
}

/// 画像の URL を返す。VALORANT なしでも使えるよう、公開データから直接取得する。
#[tauri::command]
async fn image_catalog(state: State<'_, AppState>) -> Result<ImageCatalog, String> {
    state.images.get_or_try_init(images::fetch).await.cloned().map_err(|error| error.to_string())
}

#[tauri::command]
async fn run_diagnostics(state: State<'_, AppState>) -> Result<Report, String> {
    Ok(diagnostics::run(state.source.as_ref()).await)
}

/// 匿名化した記録を保存し、保存先を返す。
#[tauri::command]
async fn save_recording(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let text = diagnostics::record(state.source.as_ref()).await.map_err(|error| error.to_string())?;
    let folder = app.path().app_data_dir().map_err(|error| error.to_string())?.join("recordings");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let path = folder.join(format!("recording-{}.json", now_ms()));
    std::fs::write(&path, text).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

/// マッチ成立を知らせる。作業を邪魔しないよう、ウィンドウは前面に出さない。
#[tauri::command]
fn notify_match(app: AppHandle, body: String) {
    if let Some(window) = app.get_webview_window("main") {
        if !window.is_focused().unwrap_or(false) {
            if let Err(error) = window.request_user_attention(Some(UserAttentionType::Critical)) {
                tracing::warn!(%error, "タスクバーの点滅に失敗");
            }
        }
    }
    // 未インストールの実行ファイルでは表示されないことがある。
    if let Err(error) = app.notification().builder().title("マッチが成立しました").body(body).show() {
        tracing::warn!(%error, "通知の表示に失敗");
    }
}

/// 共有画像をダウンロードフォルダへ保存し、保存先を返す。
#[tauri::command]
fn save_share_image(app: AppHandle, png_base64: String) -> Result<String, String> {
    let bytes = STANDARD.decode(png_base64).map_err(|error| error.to_string())?;
    let folder = app.path().download_dir().map_err(|error| error.to_string())?;
    let path = folder.join(format!("busters-match-{}.png", now_ms()));
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

/// 画面に収まる範囲で大きく開く。
fn fit_window(window: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = window.current_monitor()? else { return Ok(()) };
    let screen = monitor.size().to_logical::<f64>(monitor.scale_factor());
    let width = (screen.width * WINDOW_SCREEN_RATIO).min(WINDOW_MAX.0);
    let height = (screen.height * WINDOW_SCREEN_RATIO).min(WINDOW_MAX.1);
    window.set_size(LogicalSize::new(width, height))?;
    window.center()
}

/// Discord への表示を切り替える。
#[tauri::command]
fn set_discord_enabled(state: State<'_, AppState>, enabled: bool) {
    let _ = state.discord.lock().expect("discord poisoned").send(discord::Message::SetEnabled(enabled));
}

#[tauri::command]
fn set_discord_options(state: State<'_, AppState>, options: PresenceOptions) {
    *state.discord_options.lock().expect("discord options poisoned") = options;
    push_presence(&state);
}

#[tauri::command]
fn discord_connected(state: State<'_, AppState>) -> bool {
    state.discord_connected.load(Ordering::Relaxed)
}

/// 招待コードを発行、または無効にして、現在のコードを返す。利用者の操作でだけ呼ぶ。
#[tauri::command]
async fn set_party_invite(state: State<'_, AppState>, enabled: bool) -> Result<Option<String>, String> {
    let to_text = |error: vry_core::error::Error| error.to_string();
    let source = state.source.clone();
    let own = source.own_puuid().await.map_err(to_text)?;
    let player = source.party_player(&own).await.map_err(to_text)?;
    let id = player.as_ref().and_then(party::current_party_id).ok_or("パーティが見つかりません")?;
    let reply = source.set_party_invite(&id, enabled).await.map_err(to_text)?;
    Ok(reply.as_ref().and_then(party::parse).and_then(|party| party.invite_code))
}

async fn shop_catalog(state: &AppState) -> Result<&ShopCatalog, String> {
    state.shop_catalog.get_or_try_init(shop::fetch_catalog).await.map_err(|error| error.to_string())
}

/// ストアを読み、出現を記録する。新しい入れ替えでウィッシュリストの品があれば知らせる。
#[tauri::command]
async fn load_shop(app: AppHandle, state: State<'_, AppState>) -> Result<ShopView, String> {
    let to_text = |error: vry_core::error::Error| error.to_string();
    let source = state.source.clone();
    let own = source.own_puuid().await.map_err(to_text)?;
    let (storefront, wallet) = tokio::try_join!(source.storefront(&own), source.wallet(&own)).map_err(to_text)?;
    let storefront = storefront.ok_or("ストアの応答がありません")?;
    let wallet = wallet.unwrap_or_default();
    let catalog = shop_catalog(&state).await?;

    let remaining = storefront
        .pointer("/SkinsPanelLayout/SingleItemOffersRemainingDurationInSeconds")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    // 入れ替わる時刻は同じ回なら変わらないので、回の番号にする。時間単位へ丸めて秒の誤差を吸収する。
    let rotation = (now_ms() / 1000 + remaining + 1800) / 3600;
    let ids = shop::offered_ids(&storefront);
    let mut store = open_store(&state)?;
    let (seen, is_new) = store.record_shop(&ids, rotation, now_ms()).map_err(to_text)?;
    let wishlist: std::collections::HashSet<String> =
        store.wishlist().map_err(to_text)?.into_iter().map(|item| item.item_id).collect();
    let view = shop::build(&storefront, &wallet, catalog, &Personal { wishlist: &wishlist, seen: &seen });

    let hits: Vec<&str> = view.daily.iter().chain(&view.night_market).filter(|item| item.wished).map(|item| item.skin.name.as_str()).collect();
    if is_new && !hits.is_empty() {
        if let Err(error) = app.notification().builder().title("欲しいスキンがストアに出ています").body(hits.join("、")).show() {
            tracing::warn!(%error, "通知の表示に失敗");
        }
    }
    Ok(view)
}

#[tauri::command]
async fn search_skins(state: State<'_, AppState>, query: String) -> Result<Vec<Skin>, String> {
    Ok(shop_catalog(&state).await?.search(&query))
}

#[tauri::command]
fn wishlist(state: State<'_, AppState>) -> Result<Vec<WishItem>, String> {
    open_store(&state)?.wishlist().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_wish(state: State<'_, AppState>, item: WishItem, wished: bool) -> Result<(), String> {
    let store = open_store(&state)?;
    let result = if wished { store.add_wish(&item, now_ms()) } else { store.remove_wish(&item.item_id) };
    result.map_err(|error| error.to_string())
}

#[tauri::command]
fn review_notes(state: State<'_, AppState>, match_id: String) -> Result<Vec<ReviewNote>, String> {
    open_store(&state)?.notes(&match_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_review_note(
    state: State<'_, AppState>,
    match_id: String,
    note: ReviewNote,
) -> Result<(), String> {
    open_store(&state)?.save_note(&match_id, &note, now_ms()).map_err(|error| error.to_string())
}

#[tauri::command]
fn reviewed_matches(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    open_store(&state)?.reviewed_matches().map_err(|error| error.to_string())
}

#[tauri::command]
fn tag_trend(state: State<'_, AppState>) -> Result<Vec<TagCount>, String> {
    open_store(&state)?.tag_trend(TAG_WINDOW).map_err(|error| error.to_string())
}

fn start_engine(app: AppHandle, source: Arc<dyn RiotSource>, database: &std::path::Path) -> Result<(), String> {
    let store = Store::open(database).map_err(|error| error.to_string())?;
    let (sender, mut receiver) = watch::channel(AppEvent::Connecting { reason: None });

    tauri::async_runtime::spawn(Engine::new(source, Some(store)).run(sender));
    tauri::async_runtime::spawn(async move {
        while receiver.changed().await.is_ok() {
            let event = receiver.borrow_and_update().clone();
            *app.state::<AppState>().latest.lock().expect("latest event poisoned") = event.clone();
            push_presence(&app.state::<AppState>());
            let _ = app.emit(APP_EVENT, event);
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            init_logging(&data_dir);
            let database = data_dir.join("encounters.db");
            let source = create_source()?;

            let discord_connected = Arc::new(AtomicBool::new(false));
            app.manage(AppState {
                latest: Mutex::new(AppEvent::Connecting { reason: None }),
                database: database.clone(),
                source: source.clone(),
                images: tokio::sync::OnceCell::new(),
                discord: Mutex::new(discord::spawn(discord_connected.clone())),
                discord_connected,
                discord_options: Mutex::new(PresenceOptions::default()),
                shop_catalog: tokio::sync::OnceCell::new(),
            });
            if let Some(window) = app.get_webview_window("main") {
                fit_window(&window)?;
            }
            start_engine(app.handle().clone(), source, &database)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            latest_event,
            recent_encounters,
            load_review,
            image_catalog,
            run_diagnostics,
            save_recording,
            notify_match,
            save_share_image,
            set_discord_enabled,
            set_discord_options,
            discord_connected,
            set_party_invite,
            load_shop,
            search_skins,
            wishlist,
            set_wish,
            review_notes,
            save_review_note,
            reviewed_matches,
            tag_trend
        ])
        .run(tauri::generate_context!())
        .expect("アプリの起動に失敗しました");
}
