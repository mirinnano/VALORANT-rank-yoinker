//! 常駐まわり。トレイ、Windows 起動時の自動起動、VALORANT の起動で前面へ出す処理、自動更新。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_updater::UpdaterExt as _;

/// 自動起動のときに付ける引数。この引数で起動したら、ウィンドウを出さずにトレイで待つ。
pub const HIDDEN_ARG: &str = "--hidden";
const SETTINGS_FILE: &str = "desktop-settings.json";
const UPDATE_EVENT: &str = "update-available";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DesktopSettings {
    /// 閉じるボタンでトレイへしまう。
    pub close_to_tray: bool,
    /// VALORANT の起動を見つけたら、ウィンドウを前面に出す。
    pub bring_to_front: bool,
    /// 起動時に更新を確かめる。
    pub check_updates: bool,
    pub recording: crate::recorder::RecordingSettings,
    /// 場面を残すホットキー。
    pub clip_hotkey: String,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            bring_to_front: true,
            check_updates: true,
            recording: crate::recorder::RecordingSettings::default(),
            clip_hotkey: "F9".to_owned(),
        }
    }
}

pub struct Desktop {
    path: PathBuf,
    settings: Mutex<DesktopSettings>,
    /// 直前に VALORANT へ接続できていたか。未接続から接続への変化だけを拾う。
    was_connected: Mutex<bool>,
}

impl Desktop {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(SETTINGS_FILE);
        let settings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, settings: Mutex::new(settings), was_connected: Mutex::new(false) }
    }

    pub fn settings(&self) -> DesktopSettings {
        self.settings.lock().expect("desktop settings poisoned").clone()
    }

    fn save(&self, settings: DesktopSettings) -> Result<(), String> {
        *self.settings.lock().expect("desktop settings poisoned") = settings.clone();
        let text = serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
        std::fs::write(&self.path, text).map_err(|error| error.to_string())
    }

    /// 接続状態の変化を受け取る。未接続から接続に変わったら `true`。
    pub fn connected_now(&self, connected: bool) -> bool {
        let mut was = self.was_connected.lock().expect("desktop state poisoned");
        let rising = connected && !*was;
        *was = connected;
        rising
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn started_hidden() -> bool {
    std::env::args().any(|arg| arg == HIDDEN_ARG)
}

/// トレイのアイコンとメニューを作る。
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    let open = MenuItem::with_id(app, "open", "busters.gg を開く", true, None::<&str>)?;
    let launch = CheckMenuItem::with_id(app, "autostart", "Windows の起動時に開始", true, autostart, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &launch, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("busters.gg")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "autostart" => {
                let enabled = !app.autolaunch().is_enabled().unwrap_or(false);
                if let Err(error) = set_autostart_inner(app, enabled) {
                    tracing::warn!(%error, "自動起動の切り替えに失敗");
                }
                let _ = launch.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn set_autostart_inner(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    let result = if enabled { launcher.enable() } else { launcher.disable() };
    result.map_err(|error| error.to_string())
}

#[derive(Serialize)]
pub struct DesktopView {
    #[serde(flatten)]
    settings: DesktopSettings,
    autostart: bool,
    version: String,
}

#[tauri::command]
pub fn desktop_settings(app: AppHandle, desktop: State<'_, Desktop>) -> DesktopView {
    DesktopView {
        settings: desktop.settings(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub fn set_desktop_settings(app: AppHandle, desktop: State<'_, Desktop>, settings: DesktopSettings) -> Result<(), String> {
    let hotkey_changed = desktop.settings().clip_hotkey != settings.clip_hotkey;
    desktop.save(settings)?;
    if hotkey_changed {
        crate::clipper::register_hotkey(&app)?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    set_autostart_inner(&app, enabled)?;
    Ok(app.autolaunch().is_enabled().unwrap_or(false))
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// GitHub の配布は一時的に 503 や 504 を返すことがある。間を空けて取り直す。
const UPDATE_RETRY_DELAYS: [std::time::Duration; 3] =
    [std::time::Duration::from_secs(2), std::time::Duration::from_secs(5), std::time::Duration::from_secs(10)];

/// 失敗の文に、GitHub 側の一時的な不調かどうかを足す。
fn update_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    let busy = ["500", "502", "503", "504", "timed out", "timeout"].iter().any(|code| text.contains(code));
    if busy {
        format!("GitHub が一時的に応答していません ({text})。時間をおいて試すか、Releases からインストーラーを入手してください")
    } else {
        text
    }
}

async fn find_update(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = app.updater().map_err(|error| error.to_string())?;
    let mut last = None;
    for delay in std::iter::once(None).chain(UPDATE_RETRY_DELAYS.iter().map(Some)) {
        if let Some(delay) = delay {
            tokio::time::sleep(*delay).await;
        }
        match updater.check().await {
            Ok(found) => return Ok(found),
            Err(error) => last = Some(error),
        }
    }
    Err(update_error(last.expect("tried at least once")))
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    Ok(find_update(&app).await?.map(|update| UpdateInfo { version: update.version.clone(), notes: update.body.clone() }))
}

/// 利用者が更新を選んだときだけ呼ぶ。署名を検証してから入れ、再起動する。
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = find_update(&app).await?.ok_or("新しい版はありません")?;
    let mut last = None;
    for delay in std::iter::once(None).chain(UPDATE_RETRY_DELAYS.iter().map(Some)) {
        if let Some(delay) = delay {
            tokio::time::sleep(*delay).await;
        }
        match update.download_and_install(|_, _| {}, || {}).await {
            Ok(()) => app.restart(),
            Err(error) => {
                tracing::warn!(%error, "更新のダウンロードに失敗");
                last = Some(error);
            }
        }
    }
    Err(update_error(last.expect("tried at least once")))
}

/// 起動時に更新を確かめ、あれば画面へ知らせる。失敗しても起動は止めない。
pub fn check_update_in_background(app: AppHandle) {
    if !app.state::<Desktop>().settings().check_updates {
        return;
    }
    tauri::async_runtime::spawn(async move {
        match find_update(&app).await {
            Ok(Some(update)) => {
                let info = UpdateInfo { version: update.version.clone(), notes: update.body.clone() };
                let _ = app.emit(UPDATE_EVENT, info);
            }
            Ok(None) => {}
            Err(error) => tracing::info!(%error, "更新を確認できません"),
        }
    });
}
