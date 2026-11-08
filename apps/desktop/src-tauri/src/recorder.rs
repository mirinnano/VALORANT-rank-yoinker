//! VALORANT の画面と音を録画する。Windows だけで動く。
//!
//! 画面は Windows Graphics Capture で VALORANT のウィンドウだけを取り込み、ハードウェアエンコードで
//! H.264 の MP4 にする。音は VALORANT のプロセスが鳴らした音だけを取り込むので、通話アプリの声は入らない。
//! どちらも OS の公開 API で、ゲームのプロセスへ書き込んだりメモリを読んだりはしない。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RecordingSettings {
    /// 試合中に録画する。容量と負荷がかかるので、初期値は切っておく。
    pub enabled: bool,
    pub audio: bool,
    pub fps: u32,
    pub bitrate_mbps: u32,
    /// 残す録画の本数。
    pub keep: usize,
    /// 録画の合計の上限。
    pub max_gb: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self { enabled: false, audio: true, fps: 60, bitrate_mbps: 15, keep: 10, max_gb: 30 }
    }
}

/// 録画を終えたときの結果。
#[derive(Debug, Clone)]
pub struct Finished {
    pub path: PathBuf,
    /// 最初のフレームを受け取った時刻。エポックミリ秒。
    pub started_ms: i64,
    pub length_ms: i64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}

/// 共有用の書き出しで重ねる 1 枚の画像。位置は画面に対する割合。
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct Layer {
    pub image: PathBuf,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// クリップの先頭から出すまでの時間。
    pub delay_ms: i64,
    pub duration_ms: i64,
}

/// 冒頭に差し込む画像。
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct Intro {
    pub image: PathBuf,
    pub duration_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(windows)]
mod imp {
    use super::{now_ms, Finished, RecordingSettings};
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};
    use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder, VideoSettingsSubType,
    };
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
        SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u32 = 2;
    const BYTES_PER_FRAME: usize = 4; // 16 bit × 2 ch
    /// 音が途切れたとみなして無音で埋めるまでの遅れ。
    const AUDIO_GAP: Duration = Duration::from_millis(60);

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    /// 画面と音のスレッドで共有する。
    struct Shared {
        encoder: Mutex<Option<VideoEncoder>>,
        /// 最初のフレームの時刻。0 ならまだ届いていない。
        first_frame_ms: AtomicI64,
        first_frame: Mutex<Option<Instant>>,
        closed: AtomicBool,
        /// フレームの最短間隔。更新間隔を OS に指定できない環境では、ここで間引く。
        frame_interval: Duration,
    }

    struct Handler {
        shared: Arc<Shared>,
        last_sent: Option<Instant>,
    }

    impl GraphicsCaptureApiHandler for Handler {
        type Flags = Arc<Shared>;
        type Error = BoxError;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self { shared: ctx.flags, last_sent: None })
        }

        fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
            // 高リフレッシュレートの画面では、設定したフレームレートまで間引く。少しの揺れは許す。
            let now = Instant::now();
            if self.last_sent.is_some_and(|last| now - last < self.shared.frame_interval.mul_f64(0.9)) {
                return Ok(());
            }
            self.last_sent = Some(now);
            if self.shared.first_frame_ms.load(Ordering::Acquire) == 0 {
                *self.shared.first_frame.lock().expect("first frame poisoned") = Some(Instant::now());
                self.shared.first_frame_ms.store(now_ms(), Ordering::Release);
            }
            if let Some(encoder) = self.shared.encoder.lock().expect("encoder poisoned").as_mut() {
                encoder.send_frame(frame)?;
            }
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            // ゲームを閉じた。エンコーダーは止めるときにまとめて閉じる。
            self.shared.closed.store(true, Ordering::Release);
            Ok(())
        }
    }

    pub struct Active {
        control: Option<CaptureControl<Handler, BoxError>>,
        shared: Arc<Shared>,
        audio_stop: Arc<AtomicBool>,
        audio: Option<JoinHandle<()>>,
        path: PathBuf,
        size: (u32, u32),
    }

    /// VALORANT のウィンドウを探す。ゲームのプロセスは開かず、タイトルだけで見分ける。
    fn find_window() -> Result<Window, String> {
        Window::enumerate()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|window| window.title().is_ok_and(|title| title.trim() == "VALORANT"))
            .ok_or_else(|| "VALORANT のウィンドウが見つかりません".to_owned())
    }

    /// VALORANT のプロセスの音だけを取り込み、エンコーダーへ渡す。
    /// 音が来ない間は無音で埋め、映像との時間をそろえる。
    fn audio_loop(shared: Arc<Shared>, process_id: u32, stop: Arc<AtomicBool>) -> Result<(), BoxError> {
        use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};
        initialize_mta().ok()?;
        let format = WaveFormat::new(16, 16, SampleType::Int, SAMPLE_RATE as usize, CHANNELS as usize, None);
        let mut client = AudioClient::new_application_loopback_client(process_id, true)?;
        client.initialize_client(&format, Direction::Capture, StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 }, None)?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;

        // 映像の最初のフレームを待ってから数え始める。
        let start = loop {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(start) = *shared.first_frame.lock().expect("first frame poisoned") {
                break start;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut queue: VecDeque<u8> = VecDeque::new();
        let mut sent_frames: u64 = 0;
        while !stop.load(Ordering::Acquire) {
            let _ = event.wait_for_event(20);
            if capture.get_next_packet_size()?.unwrap_or(0) > 0 {
                capture.read_from_device_to_deque(&mut queue)?;
            }
            let expected = (start.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64;
            let mut chunk: Vec<u8> = queue.drain(..queue.len() / BYTES_PER_FRAME * BYTES_PER_FRAME).collect();
            let behind = expected.saturating_sub(sent_frames + (chunk.len() / BYTES_PER_FRAME) as u64);
            if chunk.is_empty() && behind as f64 > AUDIO_GAP.as_secs_f64() * f64::from(SAMPLE_RATE) {
                chunk = vec![0; behind as usize * BYTES_PER_FRAME];
            }
            if chunk.is_empty() {
                continue;
            }
            sent_frames += (chunk.len() / BYTES_PER_FRAME) as u64;
            if let Some(encoder) = shared.encoder.lock().expect("encoder poisoned").as_mut() {
                encoder.send_audio_buffer(&chunk, 0)?;
            }
        }
        client.stop_stream()?;
        Ok(())
    }

    pub fn start(path: &Path, settings: &RecordingSettings) -> Result<Active, String> {
        let window = find_window()?;
        let (width, height) = (
            window.width().map_err(|error| error.to_string())?,
            window.height().map_err(|error| error.to_string())?,
        );
        let process_id = window.process_id().map_err(|error| error.to_string())?;
        // H.264 は幅と高さが偶数である必要がある。
        let (width, height) = ((width.max(2) as u32) & !1, (height.max(2) as u32) & !1);

        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(width, height)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(settings.fps)
                .bitrate(settings.bitrate_mbps.saturating_mul(1_000_000)),
            AudioSettingsBuilder::default()
                .sample_rate(SAMPLE_RATE)
                .channel_count(CHANNELS)
                .bit_per_sample(16)
                .disabled(!settings.audio),
            ContainerSettingsBuilder::default(),
            path,
        )
        .map_err(|error| error.to_string())?;

        let shared = Arc::new(Shared {
            encoder: Mutex::new(Some(encoder)),
            first_frame_ms: AtomicI64::new(0),
            first_frame: Mutex::new(None),
            closed: AtomicBool::new(false),
            frame_interval: Duration::from_secs_f64(1.0 / f64::from(settings.fps.max(1))),
        });
        // 古い Windows は枠の非表示や更新間隔の指定に対応しておらず、指定すると開始できない。
        // 対応しているものだけを指定する。
        use windows_capture::graphics_capture_api::GraphicsCaptureApi;
        let border = if GraphicsCaptureApi::is_border_settings_supported().unwrap_or(false) {
            DrawBorderSettings::WithoutBorder
        } else {
            DrawBorderSettings::Default
        };
        let cursor = if GraphicsCaptureApi::is_cursor_settings_supported().unwrap_or(false) {
            CursorCaptureSettings::WithoutCursor
        } else {
            CursorCaptureSettings::Default
        };
        let interval = if GraphicsCaptureApi::is_minimum_update_interval_supported().unwrap_or(false) {
            MinimumUpdateIntervalSettings::Custom(shared.frame_interval)
        } else {
            MinimumUpdateIntervalSettings::Default
        };
        let capture = Settings::new(
            window,
            cursor,
            border,
            SecondaryWindowSettings::Default,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            shared.clone(),
        );
        let control = Handler::start_free_threaded(capture).map_err(|error| error.to_string())?;

        let audio_stop = Arc::new(AtomicBool::new(false));
        let audio = settings.audio.then(|| {
            let (shared, stop) = (shared.clone(), audio_stop.clone());
            std::thread::spawn(move || {
                if let Err(error) = audio_loop(shared, process_id, stop) {
                    tracing::warn!(%error, "音の取り込みに失敗。映像だけを録画します");
                }
            })
        });
        Ok(Active { control: Some(control), shared, audio_stop, audio, path: path.to_owned(), size: (width, height) })
    }

    impl Active {
        /// ゲームを閉じて取り込みが終わったか。
        pub fn closed(&self) -> bool {
            self.shared.closed.load(Ordering::Acquire)
                || self.control.as_ref().is_some_and(CaptureControl::is_finished)
        }

        /// 録画を止めてファイルを閉じる。時間がかかるので、非同期の処理からは別スレッドで呼ぶ。
        pub fn stop(mut self) -> Result<Finished, String> {
            let length_ms = self
                .shared
                .first_frame
                .lock()
                .expect("first frame poisoned")
                .map_or(0, |start| start.elapsed().as_millis() as i64);
            self.audio_stop.store(true, Ordering::Release);
            if let Some(audio) = self.audio.take() {
                let _ = audio.join();
            }
            if let Some(control) = self.control.take() {
                if let Err(error) = control.stop() {
                    tracing::warn!(%error, "画面の取り込みの停止に失敗");
                }
            }
            let encoder = self.shared.encoder.lock().expect("encoder poisoned").take();
            if let Some(encoder) = encoder {
                encoder.finish().map_err(|error| error.to_string())?;
            }
            let started_ms = self.shared.first_frame_ms.load(Ordering::Acquire);
            if started_ms == 0 {
                let _ = std::fs::remove_file(&self.path);
                return Err("画面を取り込めませんでした".to_owned());
            }
            let bytes = std::fs::metadata(&self.path).map_or(0, |meta| meta.len());
            Ok(Finished {
                path: self.path.clone(),
                started_ms,
                length_ms,
                bytes,
                width: self.size.0,
                height: self.size.1,
            })
        }
    }

    /// 録画から範囲を切り出して、別の MP4 にする。Windows の動画編集 API で、なるべく再エンコードせずに切る。
    pub fn export(source: &Path, start_ms: i64, end_ms: i64, folder: &Path, name: &str) -> Result<PathBuf, String> {
        use windows::core::HSTRING;
        use windows::Foundation::TimeSpan;
        use windows::Media::Editing::{MediaClip, MediaComposition, MediaTrimmingPreference};
        use windows::Media::Transcoding::TranscodeFailureReason;
        use windows::Storage::{CreationCollisionOption, StorageFile, StorageFolder};

        let run = || -> windows::core::Result<PathBuf> {
            let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(source.as_os_str()))?.join()?;
            let clip = MediaClip::CreateFromFileAsync(&file)?.join()?;
            let total = clip.OriginalDuration()?.Duration;
            let ticks = |ms: i64| ms.saturating_mul(10_000);
            clip.SetTrimTimeFromStart(TimeSpan { Duration: ticks(start_ms).clamp(0, total) })?;
            clip.SetTrimTimeFromEnd(TimeSpan { Duration: (total - ticks(end_ms)).clamp(0, total) })?;
            let composition = MediaComposition::new()?;
            composition.Clips()?.Append(&clip)?;
            let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(folder.as_os_str()))?.join()?;
            let output = folder.CreateFileAsync(&HSTRING::from(name), CreationCollisionOption::GenerateUniqueName)?.join()?;
            let reason = composition.RenderToFileWithTrimmingPreferenceAsync(&output, MediaTrimmingPreference::Fast)?.join()?;
            if reason != TranscodeFailureReason::None {
                return Err(windows::core::Error::new(windows::core::HRESULT(-1), format!("書き出しに失敗しました（理由 {}）", reason.0)));
            }
            Ok(PathBuf::from(output.Path()?.to_string()))
        };
        run().map_err(|error| error.message().to_string())
    }

    /// 範囲を切り出し、冒頭のカードと時間差で出る画像を重ねて書き出す。重ねるので再エンコードする。
    pub fn export_composed(
        source: &Path,
        start_ms: i64,
        end_ms: i64,
        intro: Option<&super::Intro>,
        layers: &[super::Layer],
        folder: &Path,
        name: &str,
    ) -> Result<PathBuf, String> {
        use windows::core::HSTRING;
        use windows::Foundation::{Rect, TimeSpan};
        use windows::Media::Editing::{MediaClip, MediaComposition, MediaOverlay, MediaOverlayLayer, MediaTrimmingPreference};
        use windows::Media::MediaProperties::{MediaEncodingProfile, VideoEncodingQuality};
        use windows::Media::Transcoding::TranscodeFailureReason;
        use windows::Storage::{CreationCollisionOption, StorageFile, StorageFolder};

        let ticks = |ms: i64| TimeSpan { Duration: ms.saturating_mul(10_000) };
        let open = |path: &Path| StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.join();
        let run = || -> windows::core::Result<PathBuf> {
            let clip = MediaClip::CreateFromFileAsync(&open(source)?)?.join()?;
            let total = clip.OriginalDuration()?.Duration;
            clip.SetTrimTimeFromStart(TimeSpan { Duration: ticks(start_ms).Duration.clamp(0, total) })?;
            clip.SetTrimTimeFromEnd(TimeSpan { Duration: (total - ticks(end_ms).Duration).clamp(0, total) })?;
            let video = clip.GetVideoEncodingProperties()?;
            let (width, height) = (f64::from(video.Width()?), f64::from(video.Height()?));

            let composition = MediaComposition::new()?;
            let lead = intro.map_or(0, |intro| intro.duration_ms);
            if let Some(intro) = intro {
                let card = MediaClip::CreateFromImageFileAsync(&open(&intro.image)?, ticks(intro.duration_ms))?.join()?;
                composition.Clips()?.Append(&card)?;
            }
            composition.Clips()?.Append(&clip)?;

            let layer = MediaOverlayLayer::new()?;
            for spec in layers {
                let image = MediaClip::CreateFromImageFileAsync(&open(&spec.image)?, ticks(spec.duration_ms.max(1)))?.join()?;
                let overlay = MediaOverlay::Create(&image)?;
                overlay.SetPosition(Rect {
                    X: (spec.x * width) as f32,
                    Y: (spec.y * height) as f32,
                    Width: (spec.width * width) as f32,
                    Height: (spec.height * height) as f32,
                })?;
                overlay.SetDelay(ticks(lead + spec.delay_ms))?;
                overlay.SetOpacity(1.0)?;
                layer.Overlays()?.Append(&overlay)?;
            }
            composition.OverlayLayers()?.Append(&layer)?;

            // 冒頭が画像でも、元の録画と同じ大きさで書き出す。
            let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::HD1080p)?;
            let output_video = profile.Video()?;
            output_video.SetWidth(width as u32)?;
            output_video.SetHeight(height as u32)?;
            output_video.SetBitrate(video.Bitrate()?.max(8_000_000))?;

            let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(folder.as_os_str()))?.join()?;
            let output = folder.CreateFileAsync(&HSTRING::from(name), CreationCollisionOption::GenerateUniqueName)?.join()?;
            let reason = composition
                .RenderToFileWithProfileAsync(&output, MediaTrimmingPreference::Precise, &profile)?
                .join()?;
            if reason != TranscodeFailureReason::None {
                return Err(windows::core::Error::new(windows::core::HRESULT(-1), format!("書き出しに失敗しました（理由 {}）", reason.0)));
            }
            Ok(PathBuf::from(output.Path()?.to_string()))
        };
        run().map_err(|error| error.message().to_string())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Finished, RecordingSettings};
    use std::path::{Path, PathBuf};

    pub struct Active;

    pub fn start(_path: &Path, _settings: &RecordingSettings) -> Result<Active, String> {
        Err("録画は Windows でだけ使えます".to_owned())
    }

    impl Active {
        pub fn closed(&self) -> bool {
            true
        }

        pub fn stop(self) -> Result<Finished, String> {
            Err("録画は Windows でだけ使えます".to_owned())
        }
    }

    pub fn export(_source: &Path, _start_ms: i64, _end_ms: i64, _folder: &Path, _name: &str) -> Result<PathBuf, String> {
        Err("書き出しは Windows でだけ使えます".to_owned())
    }

    pub fn export_composed(
        _source: &Path,
        _start_ms: i64,
        _end_ms: i64,
        _intro: Option<&super::Intro>,
        _layers: &[super::Layer],
        _folder: &Path,
        _name: &str,
    ) -> Result<PathBuf, String> {
        Err("書き出しは Windows でだけ使えます".to_owned())
    }
}

pub use imp::Active;

pub fn start(path: &Path, settings: &RecordingSettings) -> Result<Active, String> {
    imp::start(path, settings)
}

pub fn export(source: &Path, start_ms: i64, end_ms: i64, folder: &Path, name: &str) -> Result<PathBuf, String> {
    imp::export(source, start_ms, end_ms, folder, name)
}

pub fn export_composed(
    source: &Path,
    start_ms: i64,
    end_ms: i64,
    intro: Option<&Intro>,
    layers: &[Layer],
    folder: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    imp::export_composed(source, start_ms, end_ms, intro, layers, folder, name)
}

/// 録画ファイルの名前。試合の ID は長いので、時刻で区別する。
pub fn file_name(now_ms: i64) -> String {
    format!("busters-{now_ms}.mp4")
}

pub fn ensure_dir(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(path)?;
    Ok(path.to_owned())
}

pub fn current_ms() -> i64 {
    now_ms()
}
