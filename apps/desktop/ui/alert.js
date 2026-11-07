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

const DISCORD_SETTING_KEY = "discord-presence-enabled";
const DISCORD_OPTIONS_KEY = "discord-presence-options";
const DISCORD_DEFAULTS = { rank: true, score: true, party: true, invite: false };
const DISCORD_STATUS_MS = 5000;
const discordToggle = document.getElementById("discord-presence");
const discordOptionBoxes = Object.keys(DISCORD_DEFAULTS).map((key) => [key, document.getElementById(`discord-${key}`)]);
// main.js より先に読み込まれるので、ここでは共通の invoke を使えない。
const discordInvoke = window.__TAURI__.core.invoke;

function readStored(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStored(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // 保存できなくても、今回の起動では画面の状態どおりに動く。
  }
}

function storedDiscordOptions() {
  try {
    return { ...DISCORD_DEFAULTS, ...JSON.parse(readStored(DISCORD_OPTIONS_KEY) ?? "{}") };
  } catch {
    return DISCORD_DEFAULTS;
  }
}

function sendDiscordOptions() {
  const options = Object.fromEntries(discordOptionBoxes.map(([key, box]) => [key, box.checked]));
  writeStored(DISCORD_OPTIONS_KEY, JSON.stringify(options));
  discordInvoke("set_discord_options", { options });
}

async function showDiscordStatus() {
  const status = document.getElementById("discord-status");
  if (!discordToggle.checked) {
    status.textContent = "Discord への表示は止まっています。";
    return;
  }
  const connected = await discordInvoke("discord_connected");
  status.textContent = connected ? "Discord に接続しています。" : "Discord を起動すると接続します。";
}

const storedOptions = storedDiscordOptions();
for (const [key, box] of discordOptionBoxes) {
  box.checked = storedOptions[key];
  box.addEventListener("change", sendDiscordOptions);
}
discordToggle.checked = readStored(DISCORD_SETTING_KEY) !== "false";
discordInvoke("set_discord_enabled", { enabled: discordToggle.checked });
sendDiscordOptions();
discordToggle.addEventListener("change", () => {
  discordInvoke("set_discord_enabled", { enabled: discordToggle.checked });
  writeStored(DISCORD_SETTING_KEY, String(discordToggle.checked));
  showDiscordStatus();
});
showDiscordStatus();
setInterval(showDiscordStatus, DISCORD_STATUS_MS);
