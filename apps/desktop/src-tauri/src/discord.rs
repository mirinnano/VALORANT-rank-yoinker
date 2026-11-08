//! Discord Rich Presence の送信。
//!
//! Discord の IPC は同期処理なので専用スレッドで動かす。Discord が起動して
//! いなくても、アプリの動作には影響させず、一定間隔で接続を試し直す。

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Button, Party, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use vry_core::rich_presence::{PresenceClock, PresenceView};

const CLIENT_ID: &str = "1123960212580225065";
const RETRY_INTERVAL: Duration = Duration::from_secs(15);
const PROJECT_URL: &str = "https://github.com/mirinnano/VALORANT-rank-yoinker";
/// Discord の人数表示には ID が必要。参加機能は使わないので固定値にする。
const PARTY_ID: &str = "busters-party";

pub enum Message {
    Update(Option<PresenceView>),
    SetEnabled(bool),
}

fn send(client: &mut DiscordIpcClient, view: Option<&PresenceView>) -> Result<(), String> {
    let Some(view) = view else {
        return client.clear_activity().map_err(|error| error.to_string());
    };
    let mut assets = Assets::new();
    if let Some(value) = &view.large_image {
        assets = assets.large_image(value.as_str());
    }
    if let Some(value) = &view.large_text {
        assets = assets.large_text(value.as_str());
    }
    if let Some(value) = &view.small_image {
        assets = assets.small_image(value.as_str());
    }
    if let Some(value) = &view.small_text {
        assets = assets.small_text(value.as_str());
    }
    let mut activity = Activity::new().details(view.details.as_str()).assets(assets);
    if let Some(value) = &view.state {
        activity = activity.state(value.as_str());
    }
    let mut timestamps = Timestamps::new();
    if let Some(started) = view.started_ms {
        timestamps = timestamps.start(started);
    }
    if let Some(ends) = view.ends_ms {
        timestamps = timestamps.end(ends);
    }
    if let Some((size, max)) = view.party {
        activity = activity.party(Party::new().id(PARTY_ID).size([size as i32, max as i32]));
    }
    if view.competing {
        activity = activity.activity_type(ActivityType::Competing);
    }
    let mut buttons = Vec::new();
    if let Some(url) = &view.join_url {
        buttons.push(Button::new("パーティに参加", url.as_str()));
    }
    buttons.push(Button::new("busters.gg", PROJECT_URL));
    activity = activity.timestamps(timestamps).buttons(buttons);
    client.set_activity(activity).map_err(|error| error.to_string())
}

fn connect() -> Option<DiscordIpcClient> {
    let mut client = DiscordIpcClient::new(CLIENT_ID);
    client.connect().ok()?;
    tracing::info!("Discord に接続");
    Some(client)
}

pub fn spawn(connected: Arc<AtomicBool>) -> Sender<Message> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut client: Option<DiscordIpcClient> = None;
        let mut enabled = true;
        let mut clock = PresenceClock::default();
        let mut desired: Option<PresenceView> = None;
        let mut sent: Option<Option<PresenceView>> = None;
        loop {
            match receiver.recv_timeout(RETRY_INTERVAL) {
                Ok(Message::Update(mut view)) => {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
                    clock.stamp(&mut view, now);
                    desired = view;
                }
                Ok(Message::SetEnabled(value)) => enabled = value,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if !enabled {
                if let Some(mut open) = client.take() {
                    let _ = open.clear_activity();
                    let _ = open.close();
                }
                sent = None;
                connected.store(false, Ordering::Relaxed);
                continue;
            }
            if client.is_none() {
                client = connect();
                sent = None;
            }
            connected.store(client.is_some(), Ordering::Relaxed);
            let Some(open) = client.as_mut() else { continue };
            if sent.as_ref() == Some(&desired) {
                continue;
            }
            match send(open, desired.as_ref()) {
                Ok(()) => sent = Some(desired.clone()),
                Err(error) => {
                    // Discord が閉じられた場合。次の周期で接続し直す。
                    tracing::info!(%error, "Discord への送信に失敗");
                    client = None;
                    connected.store(false, Ordering::Relaxed);
                }
            }
        }
    });
    sender
}
