const CHECK_LABELS = { pass: "正常", warn: "要確認", fail: "失敗", skip: "未実施" };
const SEVERITY = { fail: 0, warn: 1, pass: 2, skip: 3 };

const diagnostic = { text: "" };
const runButton = document.getElementById("run-diagnostics");
const copyButton = document.getElementById("copy-report");
const recordButton = document.getElementById("save-recording");
const statusLine = document.getElementById("diagnostic-status");
const results = document.getElementById("diagnostic-results");

function checkRow(check) {
  const row = element("li");
  const body = element("div");
  body.append(element("span", "", check.label), element("small", "", check.detail));
  for (const field of check.missing) body.append(element("small", "", `見つからない項目: ${field}`));
  row.append(element("span", `check-status is-${check.status}`, CHECK_LABELS[check.status]), body);
  return row;
}

function reportText(report) {
  const lines = report.checks.map((check) => {
    const missing = check.missing.map((field) => `\n    missing ${field}`).join("");
    return `[${check.status}] ${check.label}: ${check.detail}${missing}`;
  });
  return lines.join("\n");
}

function summarize(checks) {
  const count = (status) => checks.filter((check) => check.status === status).length;
  const failed = count("fail");
  const warned = count("warn");
  if (failed) return `${failed} 件の失敗があります。結果をコピーして共有してください。`;
  if (warned) return `${warned} 件の要確認があります。動作に影響しない場合もあります。`;
  return "問題は見つかりませんでした。";
}

/** 実行中は同じ操作を重ねて送らないよう、ボタンを止める。 */
async function whileBusy(message, work) {
  runButton.disabled = true;
  recordButton.disabled = true;
  statusLine.textContent = message;
  try {
    await work();
  } catch (error) {
    statusLine.textContent = `実行できませんでした: ${error}`;
  } finally {
    runButton.disabled = false;
    recordButton.disabled = false;
  }
}

runButton.addEventListener("click", () => whileBusy("診断しています", async () => {
  const report = await invoke("run_diagnostics");
  const ordered = [...report.checks].sort((a, b) => SEVERITY[a.status] - SEVERITY[b.status]);
  results.replaceChildren(...ordered.map(checkRow));
  diagnostic.text = reportText(report);
  copyButton.disabled = false;
  statusLine.textContent = summarize(report.checks);
}));

copyButton.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(diagnostic.text);
    statusLine.textContent = "結果をコピーしました";
  } catch (error) {
    statusLine.textContent = `コピーできませんでした: ${error}`;
  }
});

recordButton.addEventListener("click", () => whileBusy("記録しています。少し時間がかかります", async () => {
  const path = await invoke("save_recording");
  statusLine.textContent = `保存しました: ${path}`;
}));
