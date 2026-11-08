//! Discord に出す表示を作る。
//!
//! 友人に見える情報なので、載せるのは自分の状態だけにする。他のプレイヤーの
//! 名前やランクは入れない。

use crate::images::ImageCatalog;
use crate::model::{AppEvent, GameState, Lobby, LobbyPhase, MatchSnapshot, PartyMember, PlayerView, RankView, Relation};
use serde::Deserialize;

/// 設定画面で切り替える、Discord に出す項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct PresenceOptions {
    pub rank: bool,
    pub score: bool,
    pub party: bool,
    /// フレンド全員に見えるので、初期状態では出さない。
    pub invite: bool,
}

impl Default for PresenceOptions {
    fn default() -> Self {
        Self { rank: true, score: true, party: true, invite: false }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresenceView {
    pub details: String,
    pub state: Option<String>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// 人数と上限。Discord が「(2 of 5)」の形で出す。
    pub party: Option<(u32, u32)>,
    /// 「パーティに参加」ボタンの行き先。
    pub join_url: Option<String>,
    /// 試合中は「プレイ中」ではなく「対戦中」の表示にする。
    pub competing: bool,
    /// 経過時間を出す区間の名前。同じ名前のあいだは時計を進め続ける。
    pub timer: Option<&'static str>,
    /// 残り時間。区間の最初の値から終了時刻を決める。
    pub ends_in_ms: Option<u64>,
    /// 区間の開始時刻。[`PresenceClock`] が入れる。
    pub started_ms: Option<i64>,
    pub ends_ms: Option<i64>,
}

/// 区間が変わったときだけ時刻を更新する。スコアが変わっても時計は戻らない。
#[derive(Debug, Default)]
pub struct PresenceClock {
    current: Option<(&'static str, i64, Option<i64>)>,
}

impl PresenceClock {
    pub fn stamp(&mut self, view: &mut Option<PresenceView>, now_ms: i64) {
        let timer = view.as_ref().and_then(|view| view.timer.map(|name| (name, view.ends_in_ms)));
        self.current = timer.map(|(name, ends_in)| match self.current {
            Some(current) if current.0 == name => current,
            _ => (name, now_ms, ends_in.map(|ms| now_ms + ms as i64)),
        });
        if let Some(view) = view {
            view.started_ms = self.current.map(|current| current.1);
            view.ends_ms = self.current.and_then(|current| current.2);
        }
    }
}

fn own_member(snapshot: &MatchSnapshot) -> Option<&PartyMember> {
    snapshot.lobby.as_ref()?.party.iter().find(|member| member.is_own)
}

fn party_label(lobby: &Lobby) -> String {
    match (lobby.open, lobby.party_size > 1) {
        (true, _) => "公開パーティ".to_owned(),
        (false, true) => "パーティ".to_owned(),
        (false, false) => "ソロ".to_owned(),
    }
}

fn party_count(lobby: &Lobby) -> Option<(u32, u32)> {
    (lobby.party_size > 1 && lobby.max_party_size > 0).then_some((lobby.party_size, lobby.max_party_size))
}

fn lobby_details(lobby: Option<&Lobby>, mode: &str) -> String {
    let Some(lobby) = lobby else { return format!("ロビー - {mode}") };
    match lobby.phase {
        LobbyPhase::Queueing => format!("キュー中 - {mode}"),
        LobbyPhase::MatchFound => format!("マッチ成立 - {mode}"),
        LobbyPhase::Custom => "カスタムゲーム準備中".to_owned(),
        LobbyPhase::Ready if lobby.idle => format!("離席中 - {mode}"),
        LobbyPhase::Ready => format!("ロビー - {mode}"),
    }
}

fn own_player(snapshot: &MatchSnapshot) -> Option<&PlayerView> {
    snapshot.players.iter().find(|player| player.relation == Relation::Own)
}

fn format_rank(rank: &RankView) -> String {
    match rank.leaderboard {
        0 => format!("{} {}RR", rank.name, rank.rr),
        place => format!("{} #{place}", rank.name),
    }
}

/// 自分のランク。RR を持つプロフィールを優先し、なければ試合の表示から取る。
fn rank_text(snapshot: &MatchSnapshot) -> Option<String> {
    let profile = snapshot.profile.as_ref().map(|profile| &profile.rank).filter(|rank| rank.tier > 0);
    if let Some(rank) = profile {
        return Some(format_rank(rank));
    }
    let player = own_player(snapshot).map(|player| &player.rank).filter(|rank| rank.tier > 0)?;
    Some(player.name.to_owned())
}

/// Discord の本文は常に見えるので、ランクとパーティはここに並べる。画像の説明は
/// マウスを重ねたときや一部の表示でしか出ない。
fn join_parts(parts: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let text = parts.into_iter().flatten().collect::<Vec<_>>().join(" / ");
    (!text.is_empty()).then_some(text)
}

fn lobby_view(snapshot: &MatchSnapshot, mode: &str, images: Option<&ImageCatalog>, options: PresenceOptions) -> PresenceView {
    let lobby = snapshot.lobby.as_ref();
    let profile = snapshot.profile.as_ref();
    let tier = profile.map(|profile| profile.rank.tier).filter(|tier| options.rank && *tier > 0);
    let invite = lobby.and_then(|lobby| lobby.invite_code.as_deref()).filter(|_| options.invite);
    let party = match invite {
        Some(code) => Some(format!("招待コード {code}")),
        None => lobby.filter(|_| options.party).map(party_label),
    };
    let rank = options.rank.then(|| rank_text(snapshot)).flatten();
    let day = profile
        .filter(|profile| options.rank && profile.last_day_games > 0)
        .map(|profile| format!("24時間 {:+}RR / {}試合", profile.last_day_rr, profile.last_day_games));
    let queueing = lobby.is_some_and(|lobby| lobby.phase == LobbyPhase::Queueing);
    PresenceView {
        details: lobby_details(lobby, mode),
        state: join_parts([rank.clone(), party]),
        large_image: tier
            .and_then(|tier| images?.tiers.get(&tier)?.large.clone())
            .or_else(|| own_member(snapshot)?.card_large.clone()),
        // 本文と同じ文字を2回出さないよう、説明には直近の増減を入れる。
        large_text: day,
        party: lobby.filter(|_| options.party).and_then(party_count),
        join_url: lobby.and_then(|lobby| lobby.join_url.clone()).filter(|_| options.invite),
        timer: queueing.then_some("queue"),
        ..PresenceView::default()
    }
}

fn match_view(
    snapshot: &MatchSnapshot,
    images: Option<&ImageCatalog>,
    options: PresenceOptions,
    details: String,
    timer: &'static str,
) -> PresenceView {
    let own = own_player(snapshot);
    let agent = own.and_then(|player| player.agent.as_deref());
    let rank = options.rank.then(|| rank_text(snapshot)).flatten();
    let party = snapshot.players.iter().filter(|player| {
        matches!(player.relation, Relation::Own | Relation::Party)
    }).count() as u32;
    let large_image = snapshot.map.as_ref().and_then(|map| images?.maps.get(map)?.splash.clone());
    // Discord は画像の説明も1行として出すので、同じ文字を2回出さない。
    let party_text = options.party.then(|| if party > 1 { "パーティ" } else { "ソロ" }.to_owned());
    let map = if large_image.is_some() { None } else { snapshot.map.clone() };
    PresenceView {
        details,
        timer: Some(timer),
        state: join_parts([map, rank, party_text]),
        large_text: large_image.is_some().then(|| snapshot.map.clone()).flatten(),
        large_image,
        small_image: agent.and_then(|agent| images?.agents.get(agent)?.icon.clone()),
        small_text: agent.map(str::to_owned),
        party: (options.party && party > 1).then_some((party, 5)),
        ..PresenceView::default()
    }
}

/// 接続待ちのあいだは `None`。Discord の表示を消す。
pub fn build(event: &AppEvent, images: Option<&ImageCatalog>, options: PresenceOptions) -> Option<PresenceView> {
    let AppEvent::Snapshot(snapshot) = event else { return None };
    let mode = snapshot.mode.as_deref().unwrap_or("VALORANT");
    match snapshot.state {
        GameState::Menus => Some(lobby_view(snapshot, mode, images, options)),
        GameState::Pregame => {
            let mut view = match_view(snapshot, images, options, format!("エージェント選択 - {mode}"), "pregame");
            view.ends_in_ms = snapshot.phase_remaining_ms;
            Some(view)
        }
        GameState::InGame => {
            let details = match snapshot.score.filter(|_| options.score) {
                Some((own, enemy)) => format!("{mode} {own} - {enemy}"),
                None => mode.to_owned(),
            };
            let mut view = match_view(snapshot, images, options, details, "in_game");
            view.competing = true;
            Some(view)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::MapImages;

    fn snapshot(state: GameState) -> MatchSnapshot {
        let mut snapshot = MatchSnapshot::empty(state, Some("Competitive".to_owned()));
        snapshot.map = Some("Ascent".to_owned());
        snapshot
    }

    #[test]
    fn connecting_clears_the_presence() {
        assert_eq!(build(&AppEvent::Connecting { reason: None }, None, PresenceOptions::default()), None);
    }

    #[test]
    fn in_game_shows_the_score_and_map() {
        let mut game = snapshot(GameState::InGame);
        game.score = Some((7, 5));
        let view = build(&AppEvent::Snapshot(game), None, PresenceOptions::default()).expect("view");
        assert_eq!(view.details, "Competitive 7 - 5");
        assert_eq!(view.state.as_deref(), Some("Ascent / ソロ"));
    }

    fn lobby_view(lobby: Lobby) -> PresenceView {
        let mut menus = MatchSnapshot::empty(GameState::Menus, Some("Competitive".to_owned()));
        menus.lobby = Some(lobby);
        build(&AppEvent::Snapshot(menus), None, PresenceOptions::default()).expect("view")
    }

    fn lobby(phase: LobbyPhase) -> Lobby {
        Lobby { phase, party_size: 2, max_party_size: 5, open: false, idle: false, queue_started_ms: None, party: Vec::new(), invite_code: None, join_url: None }
    }

    #[test]
    fn lobby_shows_party_and_queue() {
        let view = lobby_view(lobby(LobbyPhase::Queueing));
        assert_eq!(view.details, "キュー中 - Competitive");
        assert_eq!(view.state.as_deref(), Some("パーティ"));
        assert_eq!(view.party, Some((2, 5)));
        assert_eq!(view.timer, Some("queue"));

        let solo = lobby_view(Lobby { party_size: 1, ..lobby(LobbyPhase::Ready) });
        assert_eq!(solo.state.as_deref(), Some("ソロ"));
        assert_eq!(solo.timer, None);

        let idle = lobby_view(Lobby { idle: true, open: true, ..lobby(LobbyPhase::Ready) });
        assert_eq!(idle.details, "離席中 - Competitive");
        assert_eq!(idle.state.as_deref(), Some("公開パーティ"));
    }

    #[test]
    fn clock_survives_score_changes_and_resets_between_phases() {
        let mut clock = PresenceClock::default();
        let timed = |details: &str, timer| {
            Some(PresenceView { details: details.into(), timer: Some(timer), ..PresenceView::default() })
        };
        let mut first = timed("1 - 0", "in_game");
        clock.stamp(&mut first, 1000);
        let mut second = timed("2 - 0", "in_game");
        clock.stamp(&mut second, 9000);
        assert_eq!(second.unwrap().started_ms, Some(1000));

        let mut none = Some(PresenceView::default());
        clock.stamp(&mut none, 10000);
        assert_eq!(none.unwrap().started_ms, None);

        let mut next = timed("0 - 0", "in_game");
        clock.stamp(&mut next, 20000);
        assert_eq!(next.unwrap().started_ms, Some(20000));
    }

    #[test]
    fn map_name_appears_once() {
        let mut images = ImageCatalog::default();
        let splash = Some("https://media.valorant-api.com/m.png".to_owned());
        images.maps.insert("Ascent".to_owned(), MapImages { splash, ..Default::default() });
        let event = AppEvent::Snapshot(snapshot(GameState::InGame));

        let with_image = build(&event, Some(&images), PresenceOptions::default()).unwrap();
        assert_eq!(with_image.large_text.as_deref(), Some("Ascent"));
        assert_eq!(with_image.state.as_deref(), Some("ソロ"));

        // 画像がなければ説明も出ないので、本文にマップ名を出す。
        let without_image = build(&event, None, PresenceOptions::default()).unwrap();
        assert_eq!(without_image.state.as_deref(), Some("Ascent / ソロ"));
        assert_eq!(without_image.large_text, None);
    }

    #[test]
    fn invite_code_is_shown_only_when_enabled() {
        let mut menus = MatchSnapshot::empty(GameState::Menus, None);
        menus.lobby = Some(Lobby { invite_code: Some("ABC123".to_owned()), ..lobby(LobbyPhase::Ready) });
        let event = AppEvent::Snapshot(menus);
        let hidden = build(&event, None, PresenceOptions::default()).unwrap();
        assert_eq!(hidden.state.as_deref(), Some("パーティ"));
        let shown = PresenceOptions { invite: true, ..PresenceOptions::default() };
        assert_eq!(build(&event, None, shown).unwrap().state.as_deref(), Some("招待コード ABC123"));
    }

    #[test]
    fn join_button_follows_the_invite_option() {
        let mut menus = MatchSnapshot::empty(GameState::Menus, None);
        let url = "https://gg.riotgames.com/VAL?joinCode=AB-CD".to_owned();
        menus.lobby = Some(Lobby { join_url: Some(url.clone()), ..lobby(LobbyPhase::Ready) });
        let event = AppEvent::Snapshot(menus);
        assert_eq!(build(&event, None, PresenceOptions::default()).unwrap().join_url, None);
        let shown = PresenceOptions { invite: true, ..PresenceOptions::default() };
        assert_eq!(build(&event, None, shown).unwrap().join_url, Some(url));
    }

    #[test]
    fn options_remove_score_and_party() {
        let mut game = snapshot(GameState::InGame);
        game.score = Some((7, 5));
        let hide = PresenceOptions { score: false, party: false, ..PresenceOptions::default() };
        let view = build(&AppEvent::Snapshot(game), None, hide).unwrap();
        assert_eq!(view.details, "Competitive");
        assert!(view.competing);
        assert_eq!(view.party, None);
    }

    #[test]
    fn pick_timer_keeps_its_first_end_time() {
        let mut clock = PresenceClock::default();
        let timed = |left| Some(PresenceView { timer: Some("pregame"), ends_in_ms: Some(left), ..PresenceView::default() });
        let mut first = timed(60_000);
        clock.stamp(&mut first, 1000);
        let mut later = timed(55_123);
        clock.stamp(&mut later, 6000);
        assert_eq!(later.unwrap().ends_ms, Some(61_000));
    }

    #[test]
    fn pregame_names_the_phase() {
        let view = build(&AppEvent::Snapshot(snapshot(GameState::Pregame)), None, PresenceOptions::default()).expect("view");
        assert_eq!(view.details, "エージェント選択 - Competitive");
    }

    fn profile(tier: u32, name: &'static str, rr: u32) -> crate::profile::Profile {
        let rank = RankView { tier, name, rr, leaderboard: 0 };
        crate::profile::Profile {
            name: None,
            rank: rank.clone(),
            peak: rank,
            peak_act: None,
            win_rate: None,
            games: 0,
            last_day_rr: 18,
            last_day_games: 2,
            history: Vec::new(),
            summary: None,
            detailed_games: 0,
            matches: Vec::new(),
            partial: false,
        }
    }

    #[test]
    fn lobby_shows_rank_and_party_in_the_visible_line() {
        let mut menus = MatchSnapshot::empty(GameState::Menus, Some("Competitive".to_owned()));
        menus.lobby = Some(lobby(LobbyPhase::Ready));
        menus.profile = Some(profile(21, "Diamond 1", 45));
        let event = AppEvent::Snapshot(menus);
        let view = build(&event, None, PresenceOptions::default()).unwrap();
        assert_eq!(view.state.as_deref(), Some("Diamond 1 45RR / パーティ"));
        assert_eq!(view.party, Some((2, 5)));
        assert_eq!(view.large_text.as_deref(), Some("24時間 +18RR / 2試合"));

        let hidden = PresenceOptions { rank: false, ..PresenceOptions::default() };
        let view = build(&event, None, hidden).unwrap();
        assert_eq!(view.state.as_deref(), Some("パーティ"));
        assert_eq!(view.large_text, None);
    }

    #[test]
    fn match_shows_rank_and_party_next_to_the_map() {
        let mut images = ImageCatalog::default();
        images.maps.insert("Ascent".to_owned(), MapImages { splash: Some("https://m.png".to_owned()), ..Default::default() });
        let mut game = snapshot(GameState::InGame);
        game.profile = Some(profile(21, "Diamond 1", 45));
        let view = build(&AppEvent::Snapshot(game), Some(&images), PresenceOptions::default()).unwrap();
        assert_eq!(view.state.as_deref(), Some("Diamond 1 45RR / ソロ"));
        assert_eq!(view.large_text.as_deref(), Some("Ascent"));
    }
}
