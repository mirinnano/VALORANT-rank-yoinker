//! 試合詳細を集計し、振り返りの材料を作る。
//!
//! 試合詳細には他のプレイヤーの名前が含まれる場合がある。ここでは名前を読まず、
//! 自分以外はエージェント名だけで扱う。

use crate::analytics;
use crate::rank;
use crate::source::GameCatalog;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

const OPENING_DEATH_MIN: u32 = 4;
const OPENING_DEATH_RATIO: f64 = 0.2;
const LOSS_STREAK_MIN: usize = 4;
const FULL_BUY_LOADOUT: i64 = 3900;
const LOW_DAMAGE: i64 = 50;
const LOW_IMPACT_MIN: usize = 3;
const MULTI_KILL: u32 = 3;
const SCORE_MARGIN: f64 = 0.15;
const MAX_HIGHLIGHTS: usize = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScoreLine {
    /// 自分以外はエージェント名。
    pub label: String,
    pub agent: Option<String>,
    pub is_own: bool,
    pub is_ally: bool,
    pub rank: &'static str,
    pub tier: u32,
    pub acs: u32,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub adr: u32,
    pub headshot_percent: Option<u32>,
    pub first_kills: u32,
    pub first_deaths: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoundLine {
    /// 1 から数える。
    pub number: u32,
    pub won: bool,
    pub kills: u32,
    pub damage: i64,
    pub died: bool,
    pub first_kill: bool,
    pub first_death: bool,
    pub loadout: i64,
    pub spent: i64,
    pub result: Option<String>,
    /// 自分だけが残ったときの相手の人数。
    pub clutch_against: Option<u32>,
    /// このラウンドの自分の行動。時間順。
    pub events: Vec<RoundEvent>,
    /// このラウンドの全員の行動。時間順。
    pub timeline: Vec<TimelineEvent>,
    /// スパイクを設置した場所。`A` などの記号。
    pub plant_site: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Own,
    Ally,
    Enemy,
}

/// 行動した人。名前は出さず、エージェントと陣営で表す。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Actor {
    pub agent: Option<String>,
    pub side: Side,
}

/// ゲーム内の座標。ミニマップへの変換は画面側で行う。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

/// その瞬間の 1 人の位置。`view` は向きをミリラジアンで表す。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Placed {
    pub actor: Actor,
    pub at: Point,
    pub view: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    Kill,
    Plant,
    Defuse,
}

/// ラウンド中の 1 つの出来事。キルのほか、設置と解除も並べる。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TimelineEvent {
    pub at_ms: i64,
    pub kind: TimelineKind,
    pub actor: Option<Actor>,
    pub target: Option<Actor>,
    pub assists: Vec<Actor>,
    pub weapon: Option<String>,
    /// 倒された場所、または設置と解除の場所。
    pub location: Option<Point>,
    /// その瞬間に記録された全員の位置。
    pub positions: Vec<Placed>,
    /// この出来事のあとに生き残っている人数。`(味方, 相手)`。
    pub alive: (u32, u32),
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoundEventKind {
    Kill,
    Assist,
    Death,
    Plant,
    Defuse,
}

/// 自分の行動。相手はエージェント名だけで表す。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoundEvent {
    pub at_ms: i64,
    pub kind: RoundEventKind,
    pub target: Option<String>,
    pub weapon: Option<String>,
}

/// 試合の見どころ。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Highlight {
    pub round: u32,
    pub title: String,
    pub detail: String,
    pub score: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingTone {
    Strength,
    Focus,
}

/// データから読み取れる事実。評価ではなく、見直す場所の提案として示す。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub tone: FindingTone,
    pub title: String,
    pub detail: String,
    pub rounds: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MatchReview {
    pub match_id: String,
    pub map: Option<String>,
    pub started_ms: i64,
    pub length_ms: i64,
    pub won: Option<bool>,
    pub own_score: u32,
    pub enemy_score: u32,
    pub own: ScoreLine,
    pub team_acs: u32,
    pub team_adr: u32,
    pub lines: Vec<ScoreLine>,
    pub rounds: Vec<RoundLine>,
    pub findings: Vec<Finding>,
    /// 見どころ。良い順。
    pub highlights: Vec<Highlight>,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn int(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn kill_time(kill: &Value) -> i64 {
    kill.get("timeSinceRoundStartMillis")
        .or_else(|| kill.get("roundTime"))
        .and_then(Value::as_i64)
        .unwrap_or(i64::MAX)
}

#[derive(Default, Clone)]
struct Tally {
    damage: i64,
    hits: i64,
    headshots: i64,
    first_kills: u32,
    first_deaths: u32,
}

/// 各ラウンドの最初のキルを `(killer, victim)` で返す。
fn opening_kill(round: &Value) -> Option<(&str, &str)> {
    list(round, "playerStats")
        .iter()
        .flat_map(|player| list(player, "kills"))
        .min_by_key(|kill| kill_time(kill))
        .and_then(|kill| Some((text(kill, "killer")?, text(kill, "victim")?)))
}

fn tally_players(rounds: &[Value]) -> HashMap<&str, Tally> {
    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    for round in rounds {
        for player in list(round, "playerStats") {
            let Some(subject) = text(player, "subject") else { continue };
            let tally = tallies.entry(subject).or_default();
            for damage in list(player, "damage") {
                tally.damage += int(damage, "damage");
                let head = int(damage, "headshots");
                tally.headshots += head;
                tally.hits += head + int(damage, "bodyshots") + int(damage, "legshots");
            }
        }
        if let Some((killer, victim)) = opening_kill(round) {
            tallies.entry(killer).or_default().first_kills += 1;
            tallies.entry(victim).or_default().first_deaths += 1;
        }
    }
    tallies
}

pub(crate) fn weapon_name(kill: &Value, catalog: &GameCatalog) -> Option<String> {
    let finishing = kill.get("finishingDamage")?;
    match text(finishing, "damageType")? {
        "Weapon" => catalog.weapons.get(&text(finishing, "damageItem")?.to_lowercase()).cloned(),
        "Ability" => Some("アビリティ".to_owned()),
        "Melee" => Some("ナイフ".to_owned()),
        "Bomb" => Some("スパイク".to_owned()),
        _ => None,
    }
}

fn round_events(round: &Value, kills: &[&Value], own: &str, labels: &Labels, catalog: &GameCatalog) -> Vec<RoundEvent> {
    let label = |subject: Option<&str>| subject.and_then(|s| labels.get(s)).cloned();
    let mut events = Vec::new();
    for kill in kills {
        let at_ms = kill_time(kill).min(i64::from(i32::MAX));
        let event = |kind, target| RoundEvent { at_ms, kind, target, weapon: weapon_name(kill, catalog) };
        if text(kill, "killer") == Some(own) {
            events.push(event(RoundEventKind::Kill, label(text(kill, "victim"))));
        } else if text(kill, "victim") == Some(own) {
            events.push(event(RoundEventKind::Death, label(text(kill, "killer"))));
        } else if list(kill, "assistants").iter().any(|assistant| assistant.as_str() == Some(own)) {
            let mut assist = event(RoundEventKind::Assist, label(text(kill, "victim")));
            assist.weapon = None;
            events.push(assist);
        }
    }
    let action = |kind, key: &str, time: &str| {
        (text(round, key) == Some(own)).then(|| RoundEvent { at_ms: int(round, time), kind, target: None, weapon: None })
    };
    events.extend(action(RoundEventKind::Plant, "bombPlanter", "plantRoundTime"));
    events.extend(action(RoundEventKind::Defuse, "bombDefuser", "defuseRoundTime"));
    events.sort_by_key(|event| event.at_ms);
    events
}

type Labels = HashMap<String, String>;

fn point(value: Option<&Value>) -> Option<Point> {
    let value = value?;
    let coordinate = |key| value.get(key).and_then(Value::as_f64).map(|v| v.round() as i64);
    Some(Point { x: coordinate("x")?, y: coordinate("y")? })
}

/// 全員の行動をラウンドの時間順に並べる。
fn round_timeline(
    round: &Value,
    kills: &[&Value],
    own: &str,
    team: Option<&str>,
    teams: &HashMap<&str, &str>,
    labels: &Labels,
    catalog: &GameCatalog,
) -> Vec<TimelineEvent> {
    let actor = |subject: Option<&str>| {
        let subject = subject?;
        let side = if subject == own {
            Side::Own
        } else if team.is_some() && teams.get(subject).copied() == team {
            Side::Ally
        } else {
            Side::Enemy
        };
        Some(Actor { agent: labels.get(subject).cloned(), side })
    };
    let placed = |locations: &[Value]| -> Vec<Placed> {
        locations
            .iter()
            .filter_map(|entry| {
                Some(Placed {
                    actor: actor(text(entry, "subject"))?,
                    at: point(entry.get("location"))?,
                    view: entry.get("viewRadians").and_then(Value::as_f64).map(|v| (v * 1000.0).round() as i64),
                })
            })
            .collect()
    };

    // ラウンドに出ている人数から、生き残りを数える。
    let subjects: Vec<&str> = list(round, "playerStats").iter().filter_map(|p| text(p, "subject")).collect();
    let is_ally = |subject: &str| team.is_some() && teams.get(subject).copied() == team;
    let mut alive_allies = subjects.iter().filter(|s| is_ally(s)).count() as u32;
    let mut alive_enemies = subjects.len() as u32 - alive_allies;

    let mut events: Vec<TimelineEvent> = Vec::new();
    for kill in kills {
        let victim = text(kill, "victim");
        if let Some(victim) = victim {
            if is_ally(victim) {
                alive_allies = alive_allies.saturating_sub(1);
            } else {
                alive_enemies = alive_enemies.saturating_sub(1);
            }
        }
        events.push(TimelineEvent {
            at_ms: kill_time(kill).min(i64::from(i32::MAX)),
            kind: TimelineKind::Kill,
            actor: actor(text(kill, "killer")),
            target: actor(victim),
            assists: list(kill, "assistants").iter().filter_map(|a| actor(a.as_str())).collect(),
            weapon: weapon_name(kill, catalog),
            location: point(kill.get("victimLocation")),
            positions: placed(list(kill, "playerLocations")),
            alive: (alive_allies, alive_enemies),
        });
    }
    let mut action = |kind, who: &str, time: &str, place: &str, others: &str| {
        let Some(subject) = text(round, who) else { return };
        let at_ms = int(round, time);
        // 設置と解除の時点の生存人数は、それより前のキルから求める。
        let alive = events
            .iter()
            .filter(|event| event.kind == TimelineKind::Kill && event.at_ms <= at_ms)
            .last()
            .map_or((alive_allies_at_start(&subjects, &is_ally), alive_enemies_at_start(&subjects, &is_ally)), |e| e.alive);
        events.push(TimelineEvent {
            at_ms,
            kind,
            actor: actor(Some(subject)),
            target: None,
            assists: Vec::new(),
            weapon: None,
            location: point(round.get(place)),
            positions: placed(list(round, others)),
            alive,
        });
    };
    action(TimelineKind::Plant, "bombPlanter", "plantRoundTime", "plantLocation", "plantPlayerLocations");
    action(TimelineKind::Defuse, "bombDefuser", "defuseRoundTime", "defuseLocation", "defusePlayerLocations");
    events.sort_by_key(|event| event.at_ms);
    events
}

fn alive_allies_at_start(subjects: &[&str], is_ally: &impl Fn(&str) -> bool) -> u32 {
    subjects.iter().filter(|s| is_ally(s)).count() as u32
}

fn alive_enemies_at_start(subjects: &[&str], is_ally: &impl Fn(&str) -> bool) -> u32 {
    subjects.len() as u32 - alive_allies_at_start(subjects, is_ally)
}

fn highlight(round: &RoundLine) -> Option<Highlight> {
    let weapon = round
        .events
        .iter()
        .filter(|event| event.kind == RoundEventKind::Kill)
        .find_map(|event| event.weapon.clone());
    let suffix = weapon.map(|name| format!("、{name}")).unwrap_or_default();
    let place = format!("第{}ラウンド", round.number);
    let scene = |score, title: String, detail: String| Some(Highlight { round: round.number, title, detail, score });

    if round.won {
        if let Some(opponents) = round.clutch_against {
            let detail = format!("{place}、{} キル{suffix}", round.kills);
            return scene(60 + opponents * 15 + round.kills * 5, format!("1v{opponents} Clutch"), detail);
        }
    }
    let detail = format!("{place}{suffix}");
    match round.kills {
        5.. => scene(100, "Ace".to_owned(), detail),
        4 => scene(70, "4K".to_owned(), detail),
        3 => scene(if round.won { 45 } else { 35 }, "3K".to_owned(), detail),
        2 if round.won && round.first_kill => scene(25, "Entry".to_owned(), detail),
        _ if round.won && round.events.iter().any(|event| event.kind == RoundEventKind::Defuse) => {
            scene(30, "Defuse".to_owned(), place)
        }
        _ => None,
    }
}

fn highlights(rounds: &[RoundLine]) -> Vec<Highlight> {
    let mut scenes: Vec<Highlight> = rounds.iter().filter_map(highlight).collect();
    scenes.sort_by(|a, b| b.score.cmp(&a.score).then(a.round.cmp(&b.round)));
    scenes.truncate(MAX_HIGHLIGHTS);
    scenes
}

fn round_line(
    round: &Value,
    index: usize,
    own: &str,
    team: Option<&str>,
    teams: &HashMap<&str, &str>,
    labels: &Labels,
    catalog: &GameCatalog,
) -> RoundLine {
    let own_stats = list(round, "playerStats").iter().find(|p| text(p, "subject") == Some(own));
    let null = Value::Null;
    let stats = own_stats.unwrap_or(&null);
    let economy = stats.get("economy").unwrap_or(&null);
    let opening = opening_kill(round);
    let died = list(round, "playerStats")
        .iter()
        .flat_map(|player| list(player, "kills"))
        .any(|kill| text(kill, "victim") == Some(own));

    RoundLine {
        number: int(round, "roundNum").max(index as i64) as u32 + 1,
        won: team.is_some() && text(round, "winningTeam") == team,
        kills: list(stats, "kills").len() as u32,
        damage: list(stats, "damage").iter().map(|d| int(d, "damage")).sum(),
        died,
        first_kill: opening.is_some_and(|(killer, _)| killer == own),
        first_death: opening.is_some_and(|(_, victim)| victim == own),
        loadout: int(economy, "loadoutValue"),
        spent: int(economy, "spent"),
        result: text(round, "roundResult").map(str::to_owned),
        clutch_against: analytics::clutch_opponents(own, team, teams, &analytics::round_kills(round)),
        events: round_events(round, &analytics::round_kills(round), own, labels, catalog),
        timeline: round_timeline(round, &analytics::round_kills(round), own, team, teams, labels, catalog),
        plant_site: text(round, "plantSite").filter(|site| !site.is_empty()).map(str::to_owned),
    }
}

fn score_line(
    player: &Value,
    own: &str,
    my_team: Option<&str>,
    rounds_played: u32,
    tallies: &HashMap<&str, Tally>,
    catalog: &GameCatalog,
) -> ScoreLine {
    let null = Value::Null;
    let subject = text(player, "subject").unwrap_or_default();
    let stats = player.get("stats").unwrap_or(&null);
    let tally = tallies.get(subject).cloned().unwrap_or_default();
    let rounds = i64::from(rounds_played.max(1));
    let is_own = subject == own;
    let agent = text(player, "characterId")
        .and_then(|id| catalog.agents.get(&id.to_lowercase()))
        .cloned();
    let tier = int(player, "competitiveTier").max(0) as u32;

    ScoreLine {
        label: if is_own { "あなた".to_owned() } else { agent.clone().unwrap_or_else(|| "Player".to_owned()) },
        agent,
        is_own,
        is_ally: text(player, "teamId") == my_team,
        rank: rank::tier_name(tier),
        tier,
        acs: (int(stats, "score") / rounds) as u32,
        kills: int(stats, "kills") as u32,
        deaths: int(stats, "deaths") as u32,
        assists: int(stats, "assists") as u32,
        adr: (tally.damage / rounds) as u32,
        headshot_percent: (tally.hits > 0).then(|| (tally.headshots * 100 / tally.hits) as u32),
        first_kills: tally.first_kills,
        first_deaths: tally.first_deaths,
    }
}

fn numbers(rounds: impl Iterator<Item = u32>) -> String {
    rounds.map(|n| n.to_string()).collect::<Vec<_>>().join("、")
}

fn longest_loss_streak(rounds: &[RoundLine]) -> Vec<u32> {
    let mut best: Vec<u32> = Vec::new();
    let mut current = Vec::new();
    for round in rounds {
        if round.won {
            current.clear();
        } else {
            current.push(round.number);
            if current.len() > best.len() {
                best = current.clone();
            }
        }
    }
    best
}

fn findings(review_rounds: &[RoundLine], own: &ScoreLine, team_acs: u32) -> Vec<Finding> {
    let mut found = Vec::new();
    let played = review_rounds.len().max(1) as f64;

    let opening: Vec<u32> = review_rounds.iter().filter(|r| r.first_death).map(|r| r.number).collect();
    let opening_count = opening.len() as u32;
    if opening_count >= OPENING_DEATH_MIN && f64::from(opening_count) / played >= OPENING_DEATH_RATIO {
        found.push(Finding {
            tone: FindingTone::Focus,
            title: format!("最初に倒された: {opening_count} ラウンド"),
            detail: format!("第 {} ラウンド。立ち位置と入るタイミングを見直せます。", numbers(opening.iter().copied())),
            rounds: opening,
        });
    }

    let streak = longest_loss_streak(review_rounds);
    if streak.len() >= LOSS_STREAK_MIN {
        found.push(Finding {
            tone: FindingTone::Focus,
            title: format!("{} 連敗の区間", streak.len()),
            detail: format!("第 {} から第 {} ラウンド。流れを変える手がなかったかを確認できます。",
                streak[0], streak[streak.len() - 1]),
            rounds: streak,
        });
    }

    let low: Vec<u32> = review_rounds
        .iter()
        .filter(|r| !r.won && r.loadout >= FULL_BUY_LOADOUT && r.damage < LOW_DAMAGE)
        .map(|r| r.number)
        .collect();
    if low.len() >= LOW_IMPACT_MIN {
        found.push(Finding {
            tone: FindingTone::Focus,
            title: format!("フルバイで与ダメージ {LOW_DAMAGE} 未満: {} ラウンド", low.len()),
            detail: format!("第 {} ラウンド。装備が揃っていたのに撃ち合えなかった場面です。", numbers(low.iter().copied())),
            rounds: low,
        });
    }

    let multi: Vec<u32> = review_rounds.iter().filter(|r| r.kills >= MULTI_KILL).map(|r| r.number).collect();
    if !multi.is_empty() {
        found.push(Finding {
            tone: FindingTone::Strength,
            title: format!("{MULTI_KILL} キル以上: {} ラウンド", multi.len()),
            detail: format!("第 {} ラウンド。再現したい場面です。", numbers(multi.iter().copied())),
            rounds: multi,
        });
    }

    if team_acs > 0 {
        let ratio = f64::from(own.acs) / f64::from(team_acs);
        if ratio >= 1.0 + SCORE_MARGIN {
            found.push(Finding {
                tone: FindingTone::Strength,
                title: format!("ACS がチーム平均を上回った: {} 対 {team_acs}", own.acs),
                detail: "撃ち合いでチームを支えていました。".to_owned(),
                rounds: Vec::new(),
            });
        } else if ratio <= 1.0 - SCORE_MARGIN {
            found.push(Finding {
                tone: FindingTone::Focus,
                title: format!("ACS がチーム平均を下回った: {} 対 {team_acs}", own.acs),
                detail: "アビリティで貢献した試合なら、数字より役割で振り返ってください。".to_owned(),
                rounds: Vec::new(),
            });
        }
    }

    if own.first_kills > own.first_deaths && own.first_kills >= 3 {
        found.push(Finding {
            tone: FindingTone::Strength,
            title: format!("先手を取れた: {} 回、取られた: {} 回", own.first_kills, own.first_deaths),
            detail: "初動の撃ち合いで優位でした。".to_owned(),
            rounds: Vec::new(),
        });
    }
    found
}

/// 試合詳細を集計する。自分が参加していない試合は `None`。
pub fn analyze(own: &str, details: &Value, catalog: &GameCatalog) -> Option<MatchReview> {
    let null = Value::Null;
    let players = list(details, "players");
    let own_player = players.iter().find(|p| text(p, "subject") == Some(own))?;
    let my_team = text(own_player, "teamId");
    let info = details.get("matchInfo").unwrap_or(&null);
    let raw_rounds = list(details, "roundResults");
    let rounds_played = raw_rounds.len() as u32;
    let tallies = tally_players(raw_rounds);

    let mut lines: Vec<ScoreLine> = players
        .iter()
        .map(|player| score_line(player, own, my_team, rounds_played, &tallies, catalog))
        .collect();
    lines.sort_by(|a, b| b.is_ally.cmp(&a.is_ally).then(b.acs.cmp(&a.acs)));

    let teams: HashMap<&str, &str> =
        players.iter().filter_map(|p| Some((text(p, "subject")?, text(p, "teamId")?))).collect();
    let labels: Labels = players
        .iter()
        .filter_map(|p| {
            let agent = catalog.agents.get(&text(p, "characterId")?.to_lowercase())?;
            Some((text(p, "subject")?.to_owned(), agent.clone()))
        })
        .collect();
    let rounds: Vec<RoundLine> = raw_rounds
        .iter()
        .enumerate()
        .map(|(index, round)| round_line(round, index, own, my_team, &teams, &labels, catalog))
        .collect();

    let allies: Vec<&ScoreLine> = lines.iter().filter(|line| line.is_ally).collect();
    let average = |field: fn(&ScoreLine) -> u32| {
        allies.iter().map(|line| field(line)).sum::<u32>() / (allies.len().max(1) as u32)
    };
    let team_acs = average(|line| line.acs);
    let team_adr = average(|line| line.adr);

    let own_line = lines.iter().find(|line| line.is_own)?.clone();
    let wins = |won: bool| rounds.iter().filter(|round| round.won == won).count() as u32;
    let own_team = list(details, "teams").iter().find(|team| text(team, "teamId") == my_team);

    Some(MatchReview {
        match_id: text(info, "matchId").unwrap_or_default().to_owned(),
        map: text(info, "mapId").and_then(|id| catalog.maps.get(&id.to_lowercase())).cloned(),
        started_ms: int(info, "gameStartMillis"),
        length_ms: int(info, "gameLengthMillis"),
        won: own_team.and_then(|team| team.get("won")).and_then(Value::as_bool),
        own_score: wins(true),
        enemy_score: wins(false),
        findings: findings(&rounds, &own_line, team_acs),
        highlights: highlights(&rounds),
        own: own_line,
        team_acs,
        team_adr,
        lines,
        rounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> GameCatalog {
        let mut catalog = GameCatalog::default();
        catalog.agents.insert("a-jett".into(), "Jett".into());
        catalog.agents.insert("a-sage".into(), "Sage".into());
        catalog.agents.insert("a-omen".into(), "Omen".into());
        catalog
    }

    fn player(subject: &str, team: &str, agent: &str, score: u32) -> Value {
        json!({"subject": subject, "gameName": format!("Real{subject}"), "teamId": team,
               "characterId": agent, "competitiveTier": 15,
               "stats": {"score": score, "kills": 10, "deaths": 8, "assists": 3}})
    }

    fn round(number: u32, winner: &str, first: (&str, &str), damage: i64, loadout: i64) -> Value {
        json!({
            "roundNum": number, "winningTeam": winner, "roundResult": "Eliminated",
            "playerStats": [
                {"subject": first.0, "kills": [
                    {"killer": first.0, "victim": first.1, "timeSinceRoundStartMillis": 1000}
                ]},
                {"subject": "me", "damage": [{"damage": damage, "headshots": 1, "bodyshots": 3, "legshots": 0}],
                 "economy": {"loadoutValue": loadout, "spent": 3000}, "kills": []}
            ]
        })
    }

    fn details(rounds: Vec<Value>) -> Value {
        json!({
            "matchInfo": {"mapId": "/m"},
            "players": [
                player("me", "Blue", "a-jett", 4000),
                player("mate", "Blue", "a-sage", 2000),
                player("foe", "Red", "a-omen", 3000)
            ],
            "teams": [{"teamId": "Blue", "won": false}],
            "roundResults": rounds
        })
    }

    #[test]
    fn computes_scores_and_opening_kills() {
        let rounds = vec![
            round(0, "Blue", ("me", "foe"), 150, 4000),
            round(1, "Red", ("foe", "me"), 30, 4000),
        ];
        let r = analyze("me", &details(rounds), &catalog()).unwrap();
        assert_eq!(r.own.acs, 2000);
        assert_eq!(r.own.adr, 90);
        assert_eq!(r.own.headshot_percent, Some(25));
        assert_eq!((r.own.first_kills, r.own.first_deaths), (1, 1));
        assert_eq!((r.own_score, r.enemy_score), (1, 1));
        assert_eq!(r.won, Some(false));
        assert_eq!(r.team_acs, 1500);
        assert!(r.rounds[0].first_kill && !r.rounds[0].died);
        assert!(r.rounds[1].first_death && r.rounds[1].died);
    }

    #[test]
    fn others_are_labelled_by_agent_only() {
        let r = analyze("me", &details(vec![round(0, "Blue", ("me", "foe"), 1, 1)]), &catalog()).unwrap();
        let labels: Vec<_> = r.lines.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, ["あなた", "Sage", "Omen"]);
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("Realfoe") && !json.contains("Realmate"));
    }

    #[test]
    fn match_without_own_player_is_none() {
        assert!(analyze("nobody", &details(Vec::new()), &catalog()).is_none());
    }

    #[test]
    fn empty_rounds_do_not_divide_by_zero() {
        let r = analyze("me", &details(Vec::new()), &catalog()).unwrap();
        assert_eq!(r.own_score, 0);
        assert!(r.rounds.is_empty());
    }

    fn line(number: u32, won: bool, kills: u32) -> RoundLine {
        RoundLine {
            number, won, kills, damage: 0, died: false, first_kill: false, first_death: false,
            loadout: 0, spent: 0, result: None, clutch_against: None, events: Vec::new(),
            timeline: Vec::new(), plant_site: None,
        }
    }

    #[test]
    fn highlights_rank_clutches_and_aces_first() {
        let mut clutch = line(2, true, 3);
        clutch.clutch_against = Some(3);
        let rounds = [line(1, true, 3), clutch, line(3, false, 5), line(4, true, 0)];
        let scenes = highlights(&rounds);
        assert_eq!(scenes[0].title, "1v3 Clutch");
        assert_eq!(scenes[1].title, "Ace");
        assert_eq!(scenes[2].title, "3K");
    }

    #[test]
    fn a_lost_clutch_is_not_a_highlight() {
        let mut lost = line(1, false, 1);
        lost.clutch_against = Some(2);
        assert!(highlights(&[lost]).is_empty());
    }

    #[test]
    fn events_are_ordered_and_name_agents_only() {
        let mut value = round(0, "Blue", ("me", "foe"), 100, 4000);
        value["plantRoundTime"] = json!(5000);
        value["bombPlanter"] = json!("me");
        value["playerStats"][0]["kills"][0]["finishingDamage"] =
            json!({"damageType": "Weapon", "damageItem": "VANDAL"});
        let mut catalog = catalog();
        catalog.weapons.insert("vandal".into(), "Vandal".into());
        let r = analyze("me", &details(vec![value]), &catalog).unwrap();
        let events = &r.rounds[0].events;
        assert_eq!(events[0].kind, RoundEventKind::Kill);
        assert_eq!(events[0].target.as_deref(), Some("Omen"));
        assert_eq!(events[0].weapon.as_deref(), Some("Vandal"));
        assert_eq!(events[1].kind, RoundEventKind::Plant);
        assert!(!serde_json::to_string(&r).unwrap().contains("Realfoe"));
    }

    #[test]
    fn opening_kill_uses_the_earliest_time() {
        let value = json!({"playerStats": [
            {"kills": [{"killer": "late", "victim": "x", "timeSinceRoundStartMillis": 9000}]},
            {"kills": [{"killer": "early", "victim": "y", "roundTime": 2000}]}
        ]});
        assert_eq!(opening_kill(&value), Some(("early", "y")));
    }

    #[test]
    fn repeated_opening_deaths_become_a_focus_with_rounds() {
        let rounds: Vec<Value> =
            (0..6).map(|n| round(n, "Blue", ("foe", "me"), 100, 1000)).collect();
        let r = analyze("me", &details(rounds), &catalog()).unwrap();
        let finding = r.findings.iter().find(|f| f.title.contains("最初に倒された")).unwrap();
        assert_eq!(finding.tone, FindingTone::Focus);
        assert_eq!(finding.rounds, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn three_opening_deaths_are_not_flagged() {
        let mut rounds: Vec<Value> =
            (0..3).map(|n| round(n, "Blue", ("foe", "me"), 100, 1000)).collect();
        rounds.extend((3..10).map(|n| round(n, "Blue", ("me", "foe"), 100, 1000)));
        let r = analyze("me", &details(rounds), &catalog()).unwrap();
        assert!(!r.findings.iter().any(|f| f.title.contains("最初に倒された")));
    }

    #[test]
    fn loss_streak_uses_the_longest_run() {
        let winners = ["Red", "Red", "Blue", "Red", "Red", "Red", "Red", "Blue"];
        let rounds: Vec<Value> = winners
            .iter()
            .enumerate()
            .map(|(n, w)| round(n as u32, w, ("me", "foe"), 100, 1000))
            .collect();
        let r = analyze("me", &details(rounds), &catalog()).unwrap();
        let finding = r.findings.iter().find(|f| f.title.contains("連敗")).unwrap();
        assert_eq!(finding.rounds, [4, 5, 6, 7]);
    }

    #[test]
    fn low_damage_full_buys_are_listed_but_won_rounds_are_not() {
        let rounds = vec![
            round(0, "Red", ("foe", "mate"), 20, 4000),
            round(1, "Red", ("foe", "mate"), 20, 4000),
            round(2, "Blue", ("foe", "mate"), 20, 4000),
            round(3, "Red", ("foe", "mate"), 20, 4000),
            round(4, "Red", ("foe", "mate"), 20, 1000),
        ];
        let r = analyze("me", &details(rounds), &catalog()).unwrap();
        let finding = r.findings.iter().find(|f| f.title.contains("フルバイ")).unwrap();
        assert_eq!(finding.rounds, [1, 2, 4]);
    }

    #[test]
    fn timeline_lists_every_player_with_positions_and_alive_counts() {
        let round = json!({
            "roundNum": 0, "winningTeam": "Blue", "roundResult": "Bomb detonated",
            "bombPlanter": "me", "plantRoundTime": 40000, "plantSite": "A",
            "plantLocation": {"x": 100.4, "y": -200.0},
            "plantPlayerLocations": [{"subject": "e1", "viewRadians": 1.5, "location": {"x": 1, "y": 2}}],
            "playerStats": [
                {"subject": "me", "kills": []},
                {"subject": "a1", "kills": [
                    {"killer": "a1", "victim": "e1", "timeSinceRoundStartMillis": 12000,
                     "assistants": ["me"], "victimLocation": {"x": 5, "y": 6},
                     "playerLocations": [{"subject": "a1", "viewRadians": 0.5, "location": {"x": 3, "y": 4}}]}
                ]},
                {"subject": "e1", "kills": []},
                {"subject": "e2", "kills": [
                    {"killer": "e2", "victim": "a1", "timeSinceRoundStartMillis": 50000}
                ]}
            ]
        });
        let mut data = details(vec![round]);
        data["players"] = json!([
            player("me", "Blue", "a-jett", 200), player("a1", "Blue", "a-sage", 100),
            player("e1", "Red", "a-omen", 100), player("e2", "Red", "a-omen", 100)
        ]);
        let review = analyze("me", &data, &catalog()).unwrap();
        let line = &review.rounds[0];
        assert_eq!(line.plant_site.as_deref(), Some("A"));
        let kinds: Vec<_> = line.timeline.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [TimelineKind::Kill, TimelineKind::Plant, TimelineKind::Kill]);
        let first = &line.timeline[0];
        assert_eq!(first.actor.as_ref().map(|a| a.side), Some(Side::Ally));
        assert_eq!(first.assists[0].side, Side::Own);
        assert_eq!(first.location, Some(Point { x: 5, y: 6 }));
        assert_eq!(first.positions[0].view, Some(500));
        assert_eq!(first.alive, (2, 1));
        assert_eq!(line.timeline[1].alive, (2, 1), "設置は直前のキルの人数を引き継ぐ");
        assert_eq!(line.timeline[1].location, Some(Point { x: 100, y: -200 }));
        assert_eq!(line.timeline[2].alive, (1, 1));
        // 名前は出さない。
        assert!(!serde_json::to_string(&line.timeline).unwrap().contains("Real"));
    }
}
