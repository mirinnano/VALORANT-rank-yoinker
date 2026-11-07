//! 架空データを使い、エンジン全体の流れを確認する。

use std::sync::Arc;
use tokio::sync::{watch, Semaphore};
use vry_core::demo;
use vry_core::engine::Engine;
use vry_core::model::{AppEvent, GameState, Relation};
use vry_core::replay::{ReplaySource, Scenario};
use vry_core::review::analyze;
use vry_core::source::GameCatalog;
use vry_core::store::Store;

const MENUS: usize = 2;
const PREGAME: usize = 6;

fn engine(scenario: Scenario) -> (Engine, Arc<ReplaySource>) {
    let source = Arc::new(ReplaySource::new(scenario));
    (Engine::new(source.clone(), Some(Store::in_memory().unwrap())), source)
}

async fn advance(engine: &mut Engine, steps: usize) -> vry_core::model::MatchSnapshot {
    let mut last = None;
    for _ in 0..steps {
        last = Some(engine.step().await.unwrap());
    }
    last.unwrap()
}

#[tokio::test]
async fn walks_through_menus_pregame_and_ingame() {
    let (mut engine, _) = engine(demo::scenario());

    let menus = advance(&mut engine, MENUS).await;
    assert_eq!(menus.state, GameState::Menus);
    assert!(menus.players.is_empty());
    assert_eq!(menus.mode.as_deref(), Some("Competitive"));

    let pregame = advance(&mut engine, PREGAME).await;
    assert_eq!(pregame.state, GameState::Pregame);
    assert_eq!(pregame.players.len(), 5, "エージェント選択では味方だけが見える");
    assert_eq!(pregame.map.as_deref(), Some("Ascent"));

    let ingame = advance(&mut engine, 1).await;
    assert_eq!(ingame.state, GameState::InGame);
    assert_eq!(ingame.players.len(), 10);
    assert!(ingame.players[..5].iter().all(|p| p.relation != Relation::Enemy));
    assert!(ingame.players[5..].iter().all(|p| p.relation == Relation::Enemy));

    // 試合に進んでも、味方の鍵は変わらない。
    let key_of = |snapshot: &vry_core::model::MatchSnapshot, name: &str| {
        snapshot.players.iter().find(|p| p.display_name == name).unwrap().key
    };
    assert_eq!(key_of(&pregame, "You#0000"), key_of(&ingame, "You#0000"));
    assert_eq!(ingame.enemy_average.as_ref().unwrap().name, "Platinum 2");
}

#[tokio::test]
async fn profile_is_available_in_menus_and_refreshed_after_a_match() {
    let (mut engine, source) = engine(demo::scenario());

    let menus = advance(&mut engine, MENUS).await;
    let profile = menus.profile.expect("メニューでプロフィールが見える");
    assert_eq!(profile.name.as_deref(), Some("You#0000"));
    assert_eq!(profile.rank.name, "Platinum 1");
    assert_eq!(profile.history.len(), 20);
    assert_eq!(source.history_calls(), 1, "メニューの間は取り直さない");

    // 試合を終えてメニューへ戻ると、最新の履歴を取り直す。
    advance(&mut engine, PREGAME + 6 + 1).await;
    assert_eq!(source.history_calls(), 2);
}

#[tokio::test]
async fn live_score_updates_without_refetching_the_match() {
    let (mut engine, source) = engine(demo::scenario());
    let first = advance(&mut engine, MENUS + PREGAME + 1).await;
    let later = advance(&mut engine, 3).await;
    assert_eq!(first.score, Some((4, 3)));
    assert_eq!(later.score, Some((7, 4)));
    assert_eq!(source.coregame_calls(), 1);
}

#[tokio::test]
async fn profile_loads_when_the_app_starts_during_a_match() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS + PREGAME].clone()];
    let (mut engine, source) = engine(scenario);

    let first = engine.step().await.unwrap();
    assert_eq!(first.state, GameState::InGame);
    assert!(first.profile.is_some());

    let second = engine.step().await.unwrap();
    assert!(second.profile.is_some(), "取得済みのプロフィールを保つ");
    assert_eq!(source.history_calls(), 1);
}

#[tokio::test]
async fn failed_profile_is_not_requested_every_tick() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[0].clone()];
    scenario.fail_history = true;
    let (mut engine, source) = engine(scenario);
    advance(&mut engine, 5).await;
    assert_eq!(source.history_calls(), 1, "再試行まで待つ");
    advance(&mut engine, 10).await;
    assert_eq!(source.history_calls(), 2);
}

#[tokio::test]
async fn players_appear_before_their_ranks_finish_loading() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS + PREGAME].clone()];
    let gate = Arc::new(Semaphore::new(0));
    let source = Arc::new(ReplaySource::new(scenario).with_mmr_gate(gate.clone()));
    let (events, mut updates) = watch::channel(AppEvent::Connecting { reason: None });
    let mut engine = Engine::new(source, None).with_progress(events);
    let step = tokio::spawn(async move { engine.step().await });

    updates.changed().await.unwrap();
    let AppEvent::Snapshot(first) = updates.borrow_and_update().clone() else { panic!("snapshot") };
    assert_eq!(first.players.len(), 10, "ランクを待たずに全員を出す");
    assert!(first.players.iter().all(|player| player.loading));

    // 10 人分に加えて、自分のプロフィールでも 1 回呼ぶ。
    gate.add_permits(11);
    let finished = step.await.unwrap().unwrap();
    assert!(finished.players.iter().all(|player| !player.loading));
    assert!(finished.players.iter().any(|player| player.recent_rr.len() == 5));
}

#[tokio::test]
async fn finished_match_is_reported_after_returning_to_menus() {
    let (mut engine, _) = engine(demo::scenario());
    let before = advance(&mut engine, MENUS).await;
    assert_eq!(before.finished_match, None);
    let after = advance(&mut engine, PREGAME + 6 + 1).await;
    assert_eq!(after.finished_match.as_deref(), Some("demo-match"));
}

#[tokio::test]
async fn review_of_demo_match_hides_every_other_players_identity() {
    let scenario = demo::scenario();
    let details = &scenario.matches["history-0"];
    let review = analyze(demo::OWN, details, &GameCatalog {
        agents: scenario.agents.clone(),
        maps: scenario.maps.clone(),
    })
    .unwrap();
    assert_eq!(review.lines.len(), 10);
    let json = serde_json::to_string(&review).unwrap();
    assert!(!json.contains("Tanuki"), "試合詳細の名前は読まない");
    assert!(!json.contains("demo-enemy"), "puuid は UI へ渡さない");
}

#[tokio::test]
async fn never_requests_or_exposes_hidden_names() {
    let (mut engine, source) = engine(demo::scenario());
    let snapshot = advance(&mut engine, MENUS + PREGAME + 1).await;

    let requested = source.requested_names();
    assert!(requested.contains("demo-friend"), "パーティは本名を取得する");
    assert!(!requested.contains("demo-ally-2"));
    assert!(!requested.contains("demo-enemy-3"));

    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!json.contains("HiddenName"));
    assert!(!json.contains("SecretName"));
    let scenario = demo::scenario();
    for puuid in scenario.names.keys() {
        assert!(!json.contains(puuid.as_str()), "puuid は UI へ渡さない: {puuid}");
    }
}

#[tokio::test]
async fn hidden_levels_and_ranks_are_mapped() {
    let (mut engine, _) = engine(demo::scenario());
    let snapshot = advance(&mut engine, MENUS + PREGAME + 1).await;

    let by_name = |name: &str| snapshot.players.iter().find(|p| p.display_name == name).unwrap();
    assert_eq!(by_name("Mochi#tok").level, None);
    assert_eq!(by_name("Pixel#apac").level, None);
    assert_eq!(by_name("Friend#1111").level, Some(96));

    let own = by_name("You#0000");
    assert_eq!(own.relation, Relation::Own);
    assert_eq!(own.rank.name, "Platinum 1");
    assert_eq!(own.previous_rank.name, "Gold 3");
    assert_eq!(own.peak.name, "Platinum 2");
    assert_eq!(own.peak_act.as_deref(), Some("v26a2"));
    assert_eq!(own.party, Some(1));
    assert_eq!(by_name("Friend#1111").party, Some(1));
    assert_eq!(by_name("Nightjar#kr2").party, Some(2));
    assert_eq!(by_name("Lantern#jp1").party, None);
    assert!(own.kd.is_some() && own.headshot_percent.is_some());
}

#[tokio::test]
async fn pregame_is_refreshed_periodically() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS].clone()];
    let (mut engine, source) = engine(scenario);
    advance(&mut engine, 5).await;
    // 初回に加えて 2 ティックごとに取り直す。
    assert_eq!(source.pregame_calls(), 3);
}

#[tokio::test]
async fn ingame_is_fetched_once_per_match() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS + PREGAME].clone()];
    let (mut engine, source) = engine(scenario);
    advance(&mut engine, 5).await;
    assert_eq!(source.coregame_calls(), 1);
}

#[tokio::test]
async fn second_match_counts_previous_encounter() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS + PREGAME].clone()];
    let mut second_scenario = scenario.clone();

    let path = std::env::temp_dir().join(format!("vry-test-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let mut first = Engine::new(Arc::new(ReplaySource::new(scenario)), Some(Store::open(&path).unwrap()));
    let snapshot = first.step().await.unwrap();
    assert!(snapshot.players.iter().all(|p| p.encounters == 0));
    drop(first);

    second_scenario.coregame.as_mut().unwrap()["MatchID"] = "demo-match-2".into();
    let mut second =
        Engine::new(Arc::new(ReplaySource::new(second_scenario)), Some(Store::open(&path).unwrap()));
    let snapshot = second.step().await.unwrap();
    let friend = snapshot.players.iter().find(|p| p.relation == Relation::Party).unwrap();
    let enemy = snapshot.players.iter().find(|p| p.relation == Relation::Enemy).unwrap();
    assert_eq!(friend.encounters, 0, "パーティは記録しない");
    assert_eq!(enemy.encounters, 1);

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn missing_player_data_degrades_without_failing() {
    let mut scenario = demo::scenario();
    scenario.mmr.clear();
    scenario.updates.clear();
    let (mut engine, _) = engine(scenario);
    let snapshot = advance(&mut engine, MENUS + PREGAME + 1).await;
    assert_eq!(snapshot.players.len(), 10);
    assert!(snapshot.players.iter().all(|p| p.rank.name == "Unranked" && p.kd.is_none()));
}

#[tokio::test]
async fn empty_timeline_reports_not_ready() {
    let mut scenario = demo::scenario();
    scenario.timeline.clear();
    let (mut engine, _) = engine(scenario);
    assert!(matches!(engine.step().await, Err(vry_core::error::Error::ClientNotReady)));
}
