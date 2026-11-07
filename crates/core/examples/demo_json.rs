//! 架空データの JSON を出力する。UI をブラウザで確認するときに使う。

use serde_json::json;
use std::sync::Arc;
use vry_core::demo;
use vry_core::engine::Engine;
use vry_core::replay::ReplaySource;
use vry_core::review::analyze;
use vry_core::source::GameCatalog;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let scenario = demo::scenario();
    // 引数があれば、記録の形式でファイルへ書く。再生の確認に使う。
    if let Some(path) = std::env::args().nth(1) {
        // 実物と同じ UUID 形式の ID にしてから記録する。
        let mut text = serde_json::to_string(&scenario).expect("serialize");
        for (index, id) in scenario.names.keys().enumerate() {
            text = text.replace(id, &format!("1234abcd-0000-4000-9000-{index:012x}"));
        }
        let mut scenario: vry_core::replay::Scenario = serde_json::from_str(&text).expect("parse");
        let frame = scenario.timeline.last().cloned().unwrap_or_default();
        scenario.timeline = vec![frame];
        let text = vry_core::diagnostics::record(&ReplaySource::new(scenario)).await.expect("record");
        std::fs::write(path, text).expect("write");
        return;
    }
    let catalog = GameCatalog {
        agents: scenario.agents.clone(),
        maps: scenario.maps.clone(),
        weapons: scenario.weapons.clone(),
    };
    let review = analyze(demo::OWN, &scenario.matches["history-0"], &catalog);
    let source = Arc::new(ReplaySource::new(scenario));
    let diagnostics = vry_core::diagnostics::run(source.as_ref()).await;
    let store = vry_core::store::Store::in_memory().expect("store");
    let mut engine = Engine::new(source, Some(store));
    let mut frames = Vec::new();
    // 集計がたまるよう数周回し、最後の 1 周を UI の確認に使う。
    for _ in 0..70 {
        frames.push(engine.step().await.expect("demo step"));
    }
    let frames = frames.split_off(frames.len() - 14);
    println!("{}", json!({ "frames": frames, "review": review, "diagnostics": diagnostics }));
}
