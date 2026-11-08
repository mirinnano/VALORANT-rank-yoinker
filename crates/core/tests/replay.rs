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
async fn profile_shows_what_loaded_when_the_match_list_fails() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[0].clone()];
    scenario.fail_match_list = true;
    let (mut engine, source) = engine(scenario);
    let snapshot = engine.step().await.unwrap();
    let profile = snapshot.profile.expect("一部が取れればプロフィールを出す");
    assert!(profile.partial);
    assert!(!profile.history.is_empty(), "RR の履歴は出る");
    assert!(!profile.matches.is_empty(), "一覧がなければ RR の履歴から最近の試合を作る");
    let calls = source.history_calls();
    advance(&mut engine, 12).await;
    assert!(source.history_calls() > calls, "欠けた分を取り直す");
}

#[tokio::test]
async fn recent_matches_include_other_modes() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[0].clone()];
    scenario.match_list = Some(serde_json::json!({"History": [
        {"MatchID": "history-1", "GameStartTime": 9_999_999_999_999_i64, "QueueID": "unrated"},
        {"MatchID": "dm", "GameStartTime": 9_999_999_999_998_i64, "QueueID": "deathmatch"}
    ]}));
    let (mut engine, _source) = engine(scenario);
    advance(&mut engine, 3).await;
    let snapshot = engine.step().await.unwrap();
    let profile = snapshot.profile.unwrap();
    assert_eq!(profile.matches[0].mode.as_deref(), Some("Unrated"));
    assert_eq!(profile.matches[0].won.is_some(), true, "スコアボードのあるモードは集計する");
    assert!(profile.matches[1].kills.is_none(), "デスマッチは集計しない");
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
async fn pick_board_uses_observed_matches() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS].clone()];
    let (mut engine, _) = engine(scenario);
    let snapshot = advance(&mut engine, 1).await;
    let board = snapshot.pick_board.expect("エージェント選択で観測を出す");
    assert_eq!(board.map, "Ascent");
    assert!(board.matches >= 1);
    assert!(!board.agents.is_empty());
    assert_eq!(snapshot.phase_remaining_ms, Some(72_000));
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
        weapons: scenario.weapons.clone(),
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

#[tokio::test]
async fn menus_carry_the_party_state() {
    let mut engine = Engine::new(Arc::new(ReplaySource::new(demo::scenario())), None);
    let menus = advance(&mut engine, MENUS).await;
    let lobby = menus.lobby.expect("lobby in menus");
    assert_eq!(lobby.phase, vry_core::model::LobbyPhase::Ready);
    assert_eq!(lobby.max_party_size, 5);
    assert!(lobby.party_size >= 1);
    assert_eq!(advance(&mut engine, PREGAME).await.lobby, None);
}

#[tokio::test]
async fn result_is_rechecked_at_intervals_and_then_given_up() {
    let mut scenario = demo::scenario();
    let menus = scenario.timeline[0].clone();
    scenario.timeline.extend(std::iter::repeat_n(menus, 300));
    let (mut engine, source) = engine(scenario);

    // 架空の履歴には今の試合が載らないので、反映待ちが続く。
    advance(&mut engine, MENUS + PREGAME + 6 + 1).await;
    assert_eq!(source.history_calls(), 2, "試合直後に一度取り直す");
    advance(&mut engine, 10).await;
    assert_eq!(source.history_calls(), 2, "間隔を空ける");
    advance(&mut engine, 1).await;
    assert_eq!(source.history_calls(), 3, "間隔が過ぎたら取り直す");
    advance(&mut engine, 200).await;
    assert_eq!(source.history_calls(), 12, "上限に達したら諦める");
}

/// 新しい順の履歴。`placements` 件だけ tier のない試合を先頭に置く。
fn history_with_placements(total: usize, placements: usize) -> serde_json::Value {
    let matches: Vec<_> = (0..total)
        .map(|i| serde_json::json!({
            "MatchID": format!("page-{i}"), "MatchStartTime": 1_000_000 - i as i64,
            "TierAfterUpdate": if i < placements { 0 } else { 15 },
            "RankedRatingAfterUpdate": 40, "RankedRatingEarned": 10, "MapID": ""
        }))
        .collect();
    serde_json::json!({ "Matches": matches })
}

#[tokio::test]
async fn history_reaches_back_past_placement_matches() {
    let mut scenario = demo::scenario();
    scenario.history = Some(history_with_placements(30, 6));
    let (mut engine, source) = engine(scenario);
    let profile = advance(&mut engine, 1).await.profile.expect("profile");
    assert_eq!(profile.history.len(), 20, "tier のない試合を除いても 20 試合そろう");
    assert_eq!(source.history_calls(), 2);
    assert_eq!(profile.history.last().unwrap().match_id.as_deref(), Some("page-6"));
}

#[tokio::test]
async fn history_stops_when_the_account_has_few_matches() {
    let mut scenario = demo::scenario();
    scenario.history = Some(history_with_placements(7, 0));
    let (mut engine, source) = engine(scenario);
    let profile = advance(&mut engine, 1).await.profile.expect("profile");
    assert_eq!(profile.history.len(), 7);
    assert_eq!(source.history_calls(), 1, "足りない口座で余分に取得しない");
}

#[tokio::test]
async fn a_match_without_own_row_does_not_block_the_rest() {
    let mut scenario = demo::scenario();
    let menus = scenario.timeline[0].clone();
    scenario.timeline = vec![menus; 40];
    let ids: Vec<String> = scenario.history.as_ref().unwrap()["Matches"]
        .as_array().unwrap().iter()
        .filter_map(|m| m["MatchID"].as_str().map(str::to_owned))
        .filter(|id| scenario.matches.contains_key(id))
        .collect();
    assert!(ids.len() >= 2, "詳細のある試合が 2 件以上ないと検証にならない: {}", ids.len());
    scenario.matches.get_mut(&ids[0]).unwrap()["players"] = serde_json::json!([]);
    let (mut engine, _) = engine(scenario);
    let profile = advance(&mut engine, 30).await.profile.expect("profile");
    assert_eq!(profile.detailed_games as usize, ids.len() - 1);
}

#[tokio::test]
async fn lobby_uses_the_party_api_with_own_member_first() {
    let (mut engine, _) = engine(demo::scenario());
    let lobby = advance(&mut engine, 2).await.lobby.expect("lobby");
    assert!(lobby.party[0].is_own, "架空データではオーナーが相方でも自分が先頭");
    assert_eq!(lobby.party.iter().filter(|m| m.is_own).count(), 1);
    assert!(lobby.party[1].owner);
    assert_eq!(lobby.party[1].name.as_deref(), Some("Friend#1111"));
    assert_eq!(lobby.party[1].rank.as_ref().map(|r| r.tier), Some(14));
    assert!(lobby.party[0].card_large.as_deref().unwrap().ends_with("/largeart.png"));
    assert_eq!(lobby.invite_code, None);
    assert_eq!(lobby.join_url, None, "架空データのリンクは無効");
}

#[tokio::test]
async fn lobby_shows_only_an_unexpired_join_link() {
    let link = |expires: i64| serde_json::json!({"activities": {"p": {
        "activityId": "p", "isMember": true, "product": "valorant", "type": "party",
        "joinCode": {"isActive": true, "smartUrl": "https://gg.riotgames.com/VAL?joinCode=AB-CD", "expiresAt": expires}
    }}});
    let mut scenario = demo::scenario();
    scenario.activities = Some(link(4_102_444_800_000));
    let (mut live, _) = engine(scenario);
    let lobby = advance(&mut live, 2).await.lobby.expect("lobby");
    assert_eq!(lobby.join_url.as_deref(), Some("https://gg.riotgames.com/VAL?joinCode=AB-CD"));

    let mut expired = demo::scenario();
    expired.activities = Some(link(1));
    let (mut stale, _) = engine(expired);
    assert_eq!(advance(&mut stale, 2).await.lobby.expect("lobby").join_url, None);
}

#[tokio::test]
async fn members_outside_the_friend_list_come_from_the_party_api() {
    let mut scenario = demo::scenario();
    let extra = scenario.party.as_ref().unwrap()["Members"][0].clone();
    let mut stranger = extra;
    stranger["Subject"] = "not-a-friend".into();
    scenario.party.as_mut().unwrap()["Members"].as_array_mut().unwrap().push(stranger);
    let (mut engine, _) = engine(scenario);
    let lobby = advance(&mut engine, 1).await.lobby.expect("lobby");
    assert_eq!(lobby.party.len(), 3, "プレゼンスにない人も枠に入る");
    assert_eq!(lobby.party_size, 3);
}

#[tokio::test]
async fn lobby_falls_back_to_presence_without_the_party_api() {
    let mut scenario = demo::scenario();
    scenario.party = None;
    let (mut engine, _) = engine(scenario);
    let lobby = advance(&mut engine, 1).await.lobby.expect("lobby");
    assert_eq!(lobby.party.len(), 2);
    assert!(lobby.party[0].is_own);
    assert_eq!(lobby.party[1].card_large, None);
}

#[tokio::test]
async fn party_is_not_refetched_every_tick() {
    let mut scenario = demo::scenario();
    let menus = scenario.timeline[0].clone();
    scenario.timeline = vec![menus; 12];
    let (mut engine, source) = engine(scenario);
    advance(&mut engine, 11).await;
    assert_eq!(source.party_calls(), 3, "最初の 1 回と、5 ティックごとの読み直し");
}

#[tokio::test]
async fn a_party_event_refreshes_the_party_on_the_next_step() {
    let mut scenario = demo::scenario();
    let menus = scenario.timeline[0].clone();
    scenario.timeline = vec![menus; 6];
    let (mut engine, source) = engine(scenario);
    advance(&mut engine, 2).await;
    assert_eq!(source.party_calls(), 1);
    engine.signals().party_changed.store(true, std::sync::atomic::Ordering::Relaxed);
    advance(&mut engine, 1).await;
    assert_eq!(source.party_calls(), 2, "イベントがあれば間隔を待たずに読み直す");
    advance(&mut engine, 1).await;
    assert_eq!(source.party_calls(), 2, "合図は 1 回で消える");
}

#[tokio::test]
async fn a_pregame_event_refetches_without_waiting_for_the_tick() {
    let mut scenario = demo::scenario();
    let pregame = scenario.timeline[MENUS].clone();
    scenario.timeline = vec![pregame; 6];
    let (mut engine, source) = engine(scenario);
    advance(&mut engine, 2).await;
    let before = source.pregame_calls();
    engine.signals().pregame_changed.store(true, std::sync::atomic::Ordering::Relaxed);
    advance(&mut engine, 1).await;
    assert_eq!(source.pregame_calls(), before + 1);
}

fn live_engine(scenario: Scenario) -> Engine {
    Engine::new(Arc::new(ReplaySource::new(scenario).with_live_events()), None)
}

#[tokio::test]
async fn connected_engine_sleeps_until_an_event_wakes_it() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[0].clone(); 3];
    let mut engine = live_engine(scenario);
    let snapshot = advance(&mut engine, 2).await;
    assert!(snapshot.realtime);
    assert_eq!(engine.idle_delay(GameState::Menus), vry_core::engine::EVENT_WATCHDOG);
    assert_eq!(engine.idle_delay(GameState::InGame), vry_core::engine::EVENT_WATCHDOG);
}

#[tokio::test]
async fn disconnected_engine_keeps_polling() {
    let (mut engine, _) = engine(demo::scenario());
    let snapshot = advance(&mut engine, 2).await;
    assert!(!snapshot.realtime);
    assert_eq!(engine.idle_delay(GameState::Menus).as_secs(), 3);
}

#[tokio::test]
async fn pick_polling_stops_once_pick_events_arrive() {
    let mut scenario = demo::scenario();
    scenario.timeline = vec![scenario.timeline[MENUS].clone(); 6];
    let mut engine = live_engine(scenario);
    advance(&mut engine, 2).await;
    assert_eq!(engine.idle_delay(GameState::Pregame).as_secs(), 3, "通知が届くと分かるまでは取得を続ける");
    engine.signals().pregame_changed.store(true, std::sync::atomic::Ordering::Relaxed);
    advance(&mut engine, 1).await;
    assert_eq!(engine.idle_delay(GameState::Pregame), vry_core::engine::EVENT_WATCHDOG);
}
