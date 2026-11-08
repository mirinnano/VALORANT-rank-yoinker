//! 試合に合わせて録画を始め、終わったらクリップを作る。

use crate::desktop::Desktop;
use crate::recorder::{self, Active};
use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use vry_core::clips::{self, Clip, Recording};
use vry_core::model::{AppEvent, GameState};
use vry_core::store::Store;

const CLIPS_EVENT: &str = "clips-updated";
const MARK_EVENT: &str = "clip-marked";
/// 録画を始められなかったときに、もう一度試すまでの間隔。
const RETRY_START: Duration = Duration::from_secs(10);
/// 試合詳細が出るまで待つ回数と間隔。
const PLAN_ATTEMPTS: u32 = 12;
const PLAN_INTERVAL: Duration = Duration::from_secs(30);
const GIB: u64 = 1024 * 1024 * 1024;

struct Session {
    recorder: Active,
    match_id: Option<String>,
    map: Option<String>,
    mode: Option<String>,
    /// ホットキーを押した時刻。
    marks: Vec<i64>,
    /// スコアが増えた時刻。終わったラウンドの番号と組にする。録画と試合詳細の時刻合わせに使う。
    round_ends: Vec<(u32, i64)>,
    last_rounds: u32,
}

pub struct Clipper {
    session: Mutex<Option<Session>>,
    last_attempt: Mutex<Option<Instant>>,
    recordings: PathBuf,
    exports: PathBuf,
}

impl Clipper {
    pub fn new(videos: PathBuf) -> Self {
        let root = videos.join("busters.gg");
        Self {
            session: Mutex::new(None),
            last_attempt: Mutex::new(None),
            recordings: root.join("recordings"),
            exports: root.join("clips"),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.session.lock().expect("clip session poisoned").is_some()
    }
}

fn open_store(app: &AppHandle) -> Result<Store, String> {
    Store::open(&app.state::<AppState>().database).map_err(|error| error.to_string())
}

/// 状態の変化を受け取り、試合中だけ録画する。
pub fn on_event(app: &AppHandle, event: &AppEvent) {
    let clipper = app.state::<Clipper>();
    let settings = app.state::<Desktop>().settings().recording;
    let in_game = match event {
        AppEvent::Snapshot(snapshot) => Some((snapshot.state == GameState::InGame).then_some(snapshot)),
        // 一時的な切断では止めない。ゲームを閉じたかどうかは録画側で分かる。
        AppEvent::Connecting { .. } => None,
    };
    let closed = clipper.session.lock().expect("clip session poisoned").as_ref().is_some_and(|s| s.recorder.closed());
    match in_game {
        Some(Some(snapshot)) if !closed => {
            let mut session = clipper.session.lock().expect("clip session poisoned");
            if let Some(active) = session.as_mut() {
                // スコアが増えたら、そのラウンドが終わった時刻として残す。
                let played = snapshot.score.map_or(0, |(own, enemy)| own + enemy);
                if played > active.last_rounds {
                    active.last_rounds = played;
                    active.round_ends.push((played, recorder::current_ms()));
                }
                return;
            }
            if !settings.enabled {
                return;
            }
            let mut last = clipper.last_attempt.lock().expect("clip attempt poisoned");
            if last.is_some_and(|at| at.elapsed() < RETRY_START) {
                return;
            }
            *last = Some(Instant::now());
            let folder = match recorder::ensure_dir(&clipper.recordings) {
                Ok(folder) => folder,
                Err(error) => {
                    tracing::warn!(%error, "録画の保存先を作れません");
                    return;
                }
            };
            let path = folder.join(recorder::file_name(recorder::current_ms()));
            match recorder::start(&path, &settings) {
                Ok(active) => {
                    tracing::info!("録画を開始");
                    *session = Some(Session {
                        recorder: active,
                        match_id: snapshot.match_id.clone(),
                        map: snapshot.map.clone(),
                        mode: snapshot.mode.clone(),
                        marks: Vec::new(),
                        round_ends: Vec::new(),
                        // 試合の途中から録画を始めたときは、そこまでのラウンドを数えない。
                        last_rounds: snapshot.score.map_or(0, |(own, enemy)| own + enemy),
                    });
                    let _ = app.emit(CLIPS_EVENT, ());
                }
                Err(error) => tracing::warn!(%error, "録画を開始できません"),
            }
        }
        Some(_) => finish(app.clone()),
        None if closed => finish(app.clone()),
        None => {}
    }
}

/// 録画を閉じて保存し、クリップを作る。
fn finish(app: AppHandle) {
    let Some(session) = app.state::<Clipper>().session.lock().expect("clip session poisoned").take() else { return };
    tauri::async_runtime::spawn(async move {
        let Session { recorder, match_id, map, mode, marks, round_ends, .. } = session;
        let finished = match tauri::async_runtime::spawn_blocking(move || recorder.stop()).await {
            Ok(Ok(finished)) => finished,
            Ok(Err(error)) => {
                tracing::warn!(%error, "録画の保存に失敗");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "録画の停止に失敗");
                return;
            }
        };
        tracing::info!(length_ms = finished.length_ms, "録画を保存");
        let mut recording = Recording {
            id: 0,
            match_id: match_id.clone(),
            path: finished.path.display().to_string(),
            started_ms: finished.started_ms,
            length_ms: finished.length_ms,
            bytes: finished.bytes,
            offset_ms: 0,
            map,
            mode,
            width: Some(finished.width),
            height: Some(finished.height),
        };
        let saved = open_store(&app).and_then(|store| {
            recording.id = store.add_recording(&recording).map_err(|error| error.to_string())?;
            let placed: Vec<_> = marks
                .into_iter()
                .map(clips::manual)
                .filter_map(|plan| clips::place(&plan, &recording, false).map(|at| (plan, at)))
                .collect();
            store.add_clips(recording.id, &placed).map_err(|error| error.to_string())?;
            Ok(())
        });
        if let Err(error) = saved {
            tracing::warn!(%error, "録画の記録に失敗");
            return;
        }
        prune(&app);
        let _ = app.emit(CLIPS_EVENT, ());
        if let Some(match_id) = match_id {
            plan_when_ready(app, recording, match_id, round_ends).await;
        }
    });
}

/// 試合詳細が出るのを待って、見せ場のクリップを作る。
async fn plan_when_ready(app: AppHandle, recording: Recording, match_id: String, round_ends: Vec<(u32, i64)>) {
    for attempt in 0..PLAN_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(PLAN_INTERVAL).await;
        }
        match plan_once(&app, &recording, &match_id, &round_ends).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "クリップの作成に失敗"),
        }
    }
}

/// 試合詳細があればクリップを作って `true` を返す。`round_ends` があれば、それで
/// 録画と試合詳細の時刻を自動で合わせてから切り出す。
async fn plan_once(app: &AppHandle, recording: &Recording, match_id: &str, round_ends: &[(u32, i64)]) -> Result<bool, String> {
    let source = app.state::<AppState>().source.clone();
    let own = source.own_puuid().await.map_err(|error| error.to_string())?;
    let Some(details) = source.match_details(match_id).await.map_err(|error| error.to_string())? else {
        return Ok(false);
    };
    let catalog = source.catalog().await.map_err(|error| error.to_string())?;
    let store = open_store(app)?;
    let mut recording = recording.clone();
    if let Some(offset) = clips::estimate_offset(&details, round_ends) {
        tracing::info!(offset, "録画の時刻を合わせた");
        recording.offset_ms = offset;
        store.set_recording_offset(recording.id, offset).map_err(|error| error.to_string())?;
    }
    let placed: Vec<_> =
        clips::plan(&details, &own, &catalog).into_iter().filter_map(|plan| clips::fit(plan, &recording, true)).collect();
    store.add_clips(recording.id, &placed).map_err(|error| error.to_string())?;
    let _ = app.emit(CLIPS_EVENT, ());
    Ok(true)
}

/// 古い録画を消す。書き出したクリップは別の場所にあるので残る。
fn prune(app: &AppHandle) {
    let settings = app.state::<Desktop>().settings().recording;
    let Ok(store) = open_store(app) else { return };
    let Ok(recordings) = store.recordings() else { return };
    for id in clips::prune(&recordings, settings.keep, u64::from(settings.max_gb) * GIB) {
        if let Some(recording) = recordings.iter().find(|r| r.id == id) {
            let _ = std::fs::remove_file(&recording.path);
        }
        let _ = store.delete_recording(id);
    }
}

/// ホットキーを押した。録画中なら、その時刻を残す。
fn mark(app: &AppHandle) {
    let clipper = app.state::<Clipper>();
    let mut session = clipper.session.lock().expect("clip session poisoned");
    if let Some(session) = session.as_mut() {
        session.marks.push(recorder::current_ms());
        let _ = app.emit(MARK_EVENT, session.marks.len());
    }
}

pub fn register_hotkey(app: &AppHandle) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    shortcuts.unregister_all().map_err(|error| error.to_string())?;
    let key = app.state::<Desktop>().settings().clip_hotkey;
    if key.trim().is_empty() {
        return Ok(());
    }
    shortcuts
        .on_shortcut(key.as_str(), |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                mark(app);
            }
        })
        .map_err(|error| format!("ホットキー {key} を登録できません: {error}"))
}

#[derive(Serialize)]
pub struct RecordingView {
    #[serde(flatten)]
    recording: Recording,
    clips: Vec<Clip>,
    exists: bool,
}

#[tauri::command]
pub fn list_recordings(app: AppHandle) -> Result<Vec<RecordingView>, String> {
    let store = open_store(&app)?;
    let recordings = store.recordings().map_err(|error| error.to_string())?;
    recordings
        .into_iter()
        .map(|recording| {
            let clips = store.clips(recording.id).map_err(|error| error.to_string())?;
            let exists = std::path::Path::new(&recording.path).exists();
            Ok(RecordingView { recording, clips, exists })
        })
        .collect()
}

#[tauri::command]
pub fn recording_active(clipper: State<'_, Clipper>) -> bool {
    clipper.is_recording()
}

/// 時刻のずれを直して、自動のクリップを作り直す。
#[tauri::command]
pub async fn set_recording_offset(app: AppHandle, id: i64, offset_ms: i64) -> Result<(), String> {
    let store = open_store(&app)?;
    store.set_recording_offset(id, offset_ms).map_err(|error| error.to_string())?;
    store.clear_planned_clips(id).map_err(|error| error.to_string())?;
    let recording = store.recording(id).map_err(|error| error.to_string())?.ok_or("録画が見つかりません")?;
    if let Some(match_id) = recording.match_id.clone() {
        plan_once(&app, &recording, &match_id, &[]).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn export_clip(app: AppHandle, clip_id: i64) -> Result<String, String> {
    let store = open_store(&app)?;
    let clip = store.clip(clip_id).map_err(|error| error.to_string())?.ok_or("クリップが見つかりません")?;
    let recording =
        store.recording(clip.recording_id).map_err(|error| error.to_string())?.ok_or("録画が見つかりません")?;
    let folder = recorder::ensure_dir(&app.state::<Clipper>().exports).map_err(|error| error.to_string())?;
    let map = recording.map.clone().unwrap_or_else(|| "match".to_owned());
    let round = clip.round.map(|r| format!("-r{r}")).unwrap_or_default();
    let name = format!("busters-{map}{round}-{}.mp4", clip.title.replace(['/', '\\', ' '], ""));
    let source = PathBuf::from(&recording.path);
    let (start, end) = (clip.start_ms, clip.end_ms);
    let path = tauri::async_runtime::spawn_blocking(move || recorder::export(&source, start, end, &folder, &name))
        .await
        .map_err(|error| error.to_string())??;
    let text = path.display().to_string();
    store.set_clip_exported(clip_id, &text).map_err(|error| error.to_string())?;
    Ok(text)
}

/// 録画に映っている、試合の全員の出来事。時刻は録画の先頭からのミリ秒。
#[tauri::command]
pub async fn recording_actions(app: AppHandle, id: i64) -> Result<Vec<clips::MatchAction>, String> {
    let recording = open_store(&app)?.recording(id).map_err(|error| error.to_string())?.ok_or("録画が見つかりません")?;
    actions_of(&app, &recording).await
}

async fn actions_of(app: &AppHandle, recording: &Recording) -> Result<Vec<clips::MatchAction>, String> {
    let Some(match_id) = recording.match_id.clone() else { return Ok(Vec::new()) };
    let source = app.state::<AppState>().source.clone();
    let own = source.own_puuid().await.map_err(|error| error.to_string())?;
    let Some(details) = source.match_details(&match_id).await.map_err(|error| error.to_string())? else {
        return Ok(Vec::new());
    };
    let catalog = source.catalog().await.map_err(|error| error.to_string())?;
    Ok(clips::place_actions(clips::match_actions(&details, &own, &catalog), recording))
}

/// 画面で選んだ範囲をクリップとして残し、そのクリップを返す。書き出しは別に呼ぶ。
#[tauri::command]
pub async fn create_clip(
    app: AppHandle,
    recording_id: i64,
    round: Option<u32>,
    start_ms: i64,
    end_ms: i64,
    title: String,
) -> Result<Clip, String> {
    let store = open_store(&app)?;
    let recording =
        store.recording(recording_id).map_err(|error| error.to_string())?.ok_or("録画が見つかりません")?;
    let title: String = title.trim().chars().take(40).collect();
    // 試合詳細が取れなくても、見せ場なしでクリップは作る。
    let actions = actions_of(&app, &recording).await.unwrap_or_default();
    let chosen =
        clips::chosen(&recording, round, start_ms, end_ms, &title, &actions).ok_or("1 秒以上の範囲を選んでください")?;
    let start = chosen.1.start_ms;
    store.add_clips(recording_id, &[chosen]).map_err(|error| error.to_string())?;
    let _ = app.emit(CLIPS_EVENT, ());
    store
        .clips(recording_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|clip| clip.kind == clips::ClipKind::Manual && clip.start_ms == start)
        .ok_or_else(|| "クリップを保存できませんでした".to_owned())
}

#[tauri::command]
pub fn delete_recording(app: AppHandle, id: i64) -> Result<(), String> {
    let store = open_store(&app)?;
    if let Some(recording) = store.recording(id).map_err(|error| error.to_string())? {
        if let Err(error) = std::fs::remove_file(&recording.path) {
            tracing::warn!(%error, "録画ファイルを消せません");
        }
    }
    store.delete_recording(id).map_err(|error| error.to_string())
}

#[derive(serde::Deserialize)]
pub struct LayerInput {
    png_base64: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    delay_ms: i64,
    duration_ms: i64,
}

#[derive(serde::Deserialize)]
pub struct IntroInput {
    png_base64: String,
    duration_ms: i64,
}

/// 共有用に、オーバーレイを重ねて書き出す。画像は画面側で描いて渡す。
#[tauri::command]
pub async fn export_share_clip(
    app: AppHandle,
    clip_id: i64,
    intro: Option<IntroInput>,
    layers: Vec<LayerInput>,
) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let store = open_store(&app)?;
    let clip = store.clip(clip_id).map_err(|error| error.to_string())?.ok_or("クリップが見つかりません")?;
    let recording =
        store.recording(clip.recording_id).map_err(|error| error.to_string())?.ok_or("録画が見つかりません")?;
    let folder = recorder::ensure_dir(&app.state::<Clipper>().exports).map_err(|error| error.to_string())?;
    let work = recorder::ensure_dir(
        &app.path().app_cache_dir().map_err(|error| error.to_string())?.join(format!("share-{}", recorder::current_ms())),
    )
    .map_err(|error| error.to_string())?;
    let write = |index: usize, data: &str| -> Result<PathBuf, String> {
        let bytes = STANDARD.decode(data).map_err(|error| error.to_string())?;
        let path = work.join(format!("layer-{index}.png"));
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
        Ok(path)
    };
    let intro = match intro {
        Some(input) => Some(recorder::Intro { image: write(usize::MAX, &input.png_base64)?, duration_ms: input.duration_ms }),
        None => None,
    };
    let layers = layers
        .iter()
        .enumerate()
        .map(|(index, input)| {
            Ok(recorder::Layer {
                image: write(index, &input.png_base64)?,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
                delay_ms: input.delay_ms,
                duration_ms: input.duration_ms,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let map = recording.map.clone().unwrap_or_else(|| "match".to_owned());
    let round = clip.round.map(|r| format!("-r{r}")).unwrap_or_default();
    let name = format!("busters-{map}{round}-{}-share.mp4", clip.title.replace(['/', '\\', ' '], ""));
    let source = PathBuf::from(&recording.path);
    let (start, end) = (clip.start_ms, clip.end_ms);
    let result = tauri::async_runtime::spawn_blocking(move || {
        recorder::export_composed(&source, start, end, intro.as_ref(), &layers, &folder, &name)
    })
    .await
    .map_err(|error| error.to_string())?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(result?.display().to_string())
}

/// 書き出したファイルをエクスプローラーで選んだ状態で開く。
#[tauri::command]
pub fn reveal_file(app: AppHandle, path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    // 自分が書き出したフォルダの中だけを開く。
    let clips = &app.state::<Clipper>().exports;
    if !path.starts_with(clips) {
        return Err("書き出したクリップの場所ではありません".to_owned());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg("/select,").arg(&path).spawn().map_err(|error| error.to_string())?;
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}
