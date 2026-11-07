const ALERT_SETTING_KEY = "match-alert-enabled";
const ALERT_TONES = [660, 880, 1320];
const TONE_SECONDS = 0.14;
const TONE_VOLUME = 0.18;

const alertToggle = document.getElementById("match-alert");
const alertAudio = { context: null };

function alertEnabled() {
  try {
    return localStorage.getItem(ALERT_SETTING_KEY) !== "false";
  } catch {
    return true;
  }
}

/** 短い音を順に鳴らす。音声ファイルを同梱しなくて済むよう、その場で生成する。 */
function playAlert() {
  alertAudio.context ??= new AudioContext();
  const { context } = alertAudio;
  context.resume();
  ALERT_TONES.forEach((frequency, index) => {
    const start = context.currentTime + index * TONE_SECONDS;
    const tone = context.createOscillator();
    const gain = context.createGain();
    tone.type = "triangle";
    tone.frequency.value = frequency;
    gain.gain.setValueAtTime(TONE_VOLUME, start);
    gain.gain.exponentialRampToValueAtTime(0.001, start + TONE_SECONDS);
    tone.connect(gain).connect(context.destination);
    tone.start(start);
    tone.stop(start + TONE_SECONDS);
  });
}

function announceMatch(snapshot) {
  if (!alertToggle.checked) return;
  playAlert();
  invoke("notify_match", { body: snapshot.mode ?? "VALORANT" });
}

alertToggle.checked = alertEnabled();
alertToggle.addEventListener("change", () => {
  try {
    localStorage.setItem(ALERT_SETTING_KEY, String(alertToggle.checked));
  } catch {
    // 保存できなくても、今回の起動では画面の状態どおりに動く。
  }
});
document.getElementById("test-alert").addEventListener("click", playAlert);
