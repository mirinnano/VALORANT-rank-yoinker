//! 開発用の架空データ。実際の Riot の応答を記録したものではない。

use crate::presence::encode_private;
use crate::replay::Scenario;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const OWN: &str = "demo-own";

fn party_member(subject: &str, tier: u32, card: &str, owner: bool) -> Value {
    json!({
        "Subject": subject, "CompetitiveTier": tier, "IsOwner": owner, "IsReady": true,
        "PlayerIdentity": {"PlayerCardID": card, "PlayerTitleID": "", "AccountLevel": 120,
            "HideAccountLevel": false}
    })
}
const AGENTS: [(&str, &str); 10] = [
    ("agent-jett", "Jett"), ("agent-sage", "Sage"), ("agent-omen", "Omen"),
    ("agent-sova", "Sova"), ("agent-reyna", "Reyna"), ("agent-killjoy", "Killjoy"),
    ("agent-viper", "Viper"), ("agent-raze", "Raze"), ("agent-breach", "Breach"),
    ("agent-phoenix", "Phoenix"),
];

struct Demo {
    puuid: &'static str,
    name: &'static str,
    team: &'static str,
    tier: u32,
    level: u32,
    incognito: bool,
    hide_level: bool,
    party: &'static str,
    party_size: u32,
}

const fn p(
    puuid: &'static str, name: &'static str, team: &'static str, tier: u32, level: u32,
    incognito: bool, hide_level: bool, party: &'static str, party_size: u32,
) -> Demo {
    Demo { puuid, name, team, tier, level, incognito, hide_level, party, party_size }
}

const PLAYERS: [Demo; 10] = [
    p(OWN, "You#0000", "Blue", 15, 212, false, false, "party-a", 2),
    p("demo-friend", "Friend#1111", "Blue", 14, 96, true, false, "party-a", 2),
    p("demo-ally-1", "Lantern#jp1", "Blue", 16, 340, false, false, "solo-1", 1),
    p("demo-ally-2", "HiddenName#x", "Blue", 13, 58, true, true, "solo-2", 1),
    p("demo-ally-3", "Mochi#tok", "Blue", 15, 145, false, true, "solo-3", 1),
    p("demo-enemy-1", "Nightjar#kr2", "Red", 17, 501, false, false, "duo-b", 2),
    p("demo-enemy-2", "Quartz#sea", "Red", 16, 87, false, false, "duo-b", 2),
    p("demo-enemy-3", "SecretName#y", "Red", 15, 233, true, false, "solo-4", 1),
    p("demo-enemy-4", "Pixel#apac", "Red", 14, 19, false, true, "solo-5", 1),
    p("demo-enemy-5", "Tanuki#ryu", "Red", 18, 410, false, false, "solo-6", 1),
];

fn identity(d: &Demo) -> Value {
    json!({"AccountLevel": d.level, "Incognito": d.incognito, "HideAccountLevel": d.hide_level})
}

/// 平坦な形式と入れ子の形式を混ぜ、両方が通ることを確認する。
fn presence(d: &Demo, state: &str, nested: bool, score: (u32, u32)) -> Value {
    let (ally, enemy) = score;
    let private = if nested {
        json!({
            "matchPresenceData": {"sessionLoopState": state, "queueId": "competitive",
                "provisioningFlow": "Matchmaking",
                "partyOwnerMatchScoreAllyTeam": ally, "partyOwnerMatchScoreEnemyTeam": enemy},
            "partyPresenceData": {"partyId": d.party, "partySize": d.party_size, "maxPartySize": 5,
                "partyAccessibility": "CLOSED", "partyState": "DEFAULT"},
            "playerPresenceData": {"accountLevel": d.level, "isIdle": false},
            "isValid": true
        })
    } else {
        json!({
            "sessionLoopState": state, "queueId": "competitive", "partyId": d.party,
            "provisioningFlow": "Matchmaking",
            "partySize": d.party_size, "maxPartySize": 5, "partyAccessibility": "CLOSED",
            "partyState": "DEFAULT", "isIdle": false, "accountLevel": d.level, "isValid": true,
            "partyOwnerMatchScoreAllyTeam": ally, "partyOwnerMatchScoreEnemyTeam": enemy
        })
    };
    json!({"puuid": d.puuid, "product": "valorant", "private": encode_private(&private)})
}

fn frame(state: &str, score: (u32, u32)) -> Vec<Value> {
    PLAYERS.iter().enumerate().map(|(i, d)| presence(d, state, i % 2 == 1, score)).collect()
}

fn mmr(d: &Demo) -> Value {
    let games = 20 + d.level % 60;
    let wins = games / 2 + d.tier % 5;
    json!({"QueueSkills": {"competitive": {"SeasonalInfoBySeasonID": {
        "season-now": {
            "CompetitiveTier": d.tier, "RankedRating": (d.level * 7) % 100, "LeaderboardRank": 0,
            "NumberOfGames": games, "NumberOfWinsWithPlacements": wins,
            "WinsByTier": {d.tier.to_string(): 3}
        },
        "season-prev": {
            "CompetitiveTier": d.tier - 1, "RankedRating": 30,
            "NumberOfGames": 40, "NumberOfWinsWithPlacements": 21,
            "WinsByTier": {(d.tier + 1).to_string(): 2}
        }
    }}}})
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// 新しい試合が先頭に来る、20 試合分の架空の増減。
fn rank_history() -> Value {
    const EARNED: [i32; 20] = [18, -14, 22, 9, -21, 16, 24, -8, 12, -17, 20, 15, -19, 6, 23, -11, 14, 19, -16, 21];
    const MAPS: [&str; 5] = [
        "/Game/Maps/Ascent/Ascent", "/Game/Maps/Bonsai/Bonsai", "/Game/Maps/Triad/Triad",
        "/Game/Maps/Duality/Duality", "/Game/Maps/Port/Port",
    ];
    const HOUR: i64 = 3_600_000;
    let now = now_ms();
    let mut total = 1584; // tier 15 と RR 84 を合わせた累積値
    let matches: Vec<Value> = EARNED
        .iter()
        .enumerate()
        .map(|(i, earned)| {
            let after = total;
            total -= earned;
            json!({
                "MatchID": format!("history-{i}"),
                "MapID": MAPS[i % MAPS.len()],
                "SeasonID": "season-now",
                "MatchStartTime": now - HOUR * (i as i64 * 5 + 1),
                "TierAfterUpdate": after / 100,
                "RankedRatingAfterUpdate": after % 100,
                "RankedRatingEarned": earned,
                "AFKPenalty": 0
            })
        })
        .collect();
    json!({"Matches": matches})
}

/// 振り返り画面用の架空の試合詳細。結果は `seed` で変わる。
fn review_details(seed: usize) -> Value {
    const ROUNDS: usize = 22;
    let won_round = |round: usize| (round * 7 + seed * 3) % 5 < 3;
    let player = |index: usize, round: usize| (round + seed + index * 3) % 10;

    let results: Vec<Value> = (0..ROUNDS)
        .map(|round| {
            let first_killer = player(1, round);
            let stats: Vec<Value> = PLAYERS
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let kills: Vec<Value> = if i == first_killer {
                        // 自分が開幕で倒される場面を混ぜ、気づきの表示を確認できるようにする。
                        let victim = if i >= 5 && round % 2 == 0 { OWN } else { PLAYERS[(i + 5) % 10].puuid };
                        vec![json!({"killer": d.puuid, "victim": victim, "assistants": [],
                                    "finishingDamage": {"damageType": "Weapon", "damageItem": "weapon-vandal"},
                                    "timeSinceRoundStartMillis": 18_000 + round * 300})]
                    } else if i == 0 && round == 9 {
                        // 自分のエースを 1 回入れ、見どころの表示を確認できるようにする。
                        (5..10)
                            .map(|enemy| json!({
                                "killer": d.puuid, "victim": PLAYERS[enemy].puuid, "assistants": [],
                                "finishingDamage": {"damageType": "Weapon", "damageItem": "weapon-vandal"},
                                "timeSinceRoundStartMillis": 25_000 + enemy * 3_000
                            }))
                            .collect()
                    } else if (round + i + seed) % 4 == 0 {
                        let victim = PLAYERS[(i + 6) % 10].puuid;
                        vec![json!({"killer": d.puuid, "victim": victim, "assistants": [],
                                    "timeSinceRoundStartMillis": 40_000 + i * 500})]
                    } else {
                        Vec::new()
                    };
                    let damage = 40 + ((round + 1) * (i + 2) * 17) % 170;
                    json!({
                        "subject": d.puuid,
                        "score": 80 + damage + kills.len() * 150,
                        "kills": kills,
                        "damage": [{"damage": damage, "receiver": PLAYERS[(i + 5) % 10].puuid,
                                    "headshots": (round + i) % 3, "bodyshots": 3, "legshots": 1}],
                        "economy": {"spent": 3900 - (round % 4) * 800,
                                    "loadoutValue": 4300 - (round % 4) * 900, "remaining": 200}
                    })
                })
                .collect();
            json!({
                "roundNum": round,
                "winningTeam": if won_round(round) { "Blue" } else { "Red" },
                "roundResult": "Eliminated",
                "bombPlanter": PLAYERS[if round < 12 { 5 } else { 0 }].puuid,
                "plantRoundTime": 42_000,
                "playerStats": stats
            })
        })
        .collect();

    let blue_wins = (0..ROUNDS).filter(|round| won_round(*round)).count();
    let players: Vec<Value> = PLAYERS
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!({
                "subject": d.puuid, "gameName": d.name, "teamId": d.team,
                "characterId": AGENTS[(i + seed / 4) % 10].0, "competitiveTier": d.tier,
                "stats": {"score": 3800 + (i + seed) * 310 % 2400, "roundsPlayed": ROUNDS,
                          "kills": 9 + (i + seed) % 12, "deaths": 10 + i % 7, "assists": 2 + i % 6}
            })
        })
        .collect();

    json!({
        "matchInfo": {"mapId": "/Game/Maps/Ascent/Ascent", "queueID": "competitive",
                      "gameStartMillis": now_ms() - seed as i64 * 18_000_000, "gameLengthMillis": 2_400_000},
        "players": players,
        "teams": [
            {"teamId": "Blue", "won": blue_wins * 2 > ROUNDS, "roundsWon": blue_wins},
            {"teamId": "Red", "won": blue_wins * 2 < ROUNDS, "roundsWon": ROUNDS - blue_wins}
        ],
        "roundResults": results
    })
}

pub fn scenario() -> Scenario {
    let raw_player = |(i, d): (usize, &Demo)| {
        json!({
            "Subject": d.puuid, "TeamID": d.team, "CharacterID": AGENTS[i].0,
            "CharacterSelectionState": if i % 3 == 0 { "selected" } else { "locked" },
            "PlayerIdentity": identity(d)
        })
    };
    let allies: Vec<Value> = PLAYERS[..5].iter().enumerate().map(raw_player).collect();
    let everyone: Vec<Value> = PLAYERS.iter().enumerate().map(raw_player).collect();

    let mut mmrs = HashMap::new();
    let mut updates = HashMap::new();
    for (i, d) in PLAYERS.iter().enumerate() {
        mmrs.insert(d.puuid.to_owned(), mmr(d));
        let first = json!({
            "MatchID": "history-0", "RankedRatingEarned": 18 - (i as i32) * 6,
            "AFKPenalty": 0, "MatchStartTime": now_ms() - (i as i64 + 1) * 3_600_000 * 20
        });
        let older = (1..5).map(|age| {
            let earned = if (i + age) % 3 == 0 { -14 } else { 16 };
            json!({"MatchID": format!("demo-old-{age}"), "RankedRatingEarned": earned})
        });
        let matches: Vec<Value> = std::iter::once(first).chain(older).collect();
        updates.insert(d.puuid.to_owned(), json!({ "Matches": matches }));
    }

    let mut timeline = Vec::new();
    timeline.extend(std::iter::repeat_n(frame("MENUS", (0, 0)), 2));
    timeline.extend(std::iter::repeat_n(frame("PREGAME", (0, 0)), 6));
    timeline.extend((0..6).map(|round| frame("INGAME", (round + 4, round / 2 + 3))));

    Scenario {
        own: OWN.to_owned(),
        timeline,
        pregame: Some(json!({
            "ID": "demo-match", "MapID": "/Game/Maps/Ascent/Ascent",
            "PhaseTimeRemainingNS": 72_000_000_000_u64,
            "AllyTeam": {"TeamID": "Blue", "Players": allies}
        })),
        coregame: Some(json!({
            "MatchID": "demo-match", "MapID": "/Game/Maps/Ascent/Ascent", "Players": everyone
        })),
        names: PLAYERS.iter().map(|d| (d.puuid.to_owned(), d.name.to_owned())).collect(),
        mmr: mmrs,
        updates,
        history: Some(rank_history()),
        fail_history: false,
        party: Some(json!({
            "ID": "party-a", "Accessibility": "CLOSED", "InviteCode": "",
            "Members": [
                party_member("demo-friend", 14, "33c1f011-4eca-068c-9751-f68c788b2eee", true),
                party_member(OWN, 15, "9fb348bc-41a0-91ad-8a3e-818035c4e561", false),
            ]
        })),
        storefront: Some(json!({
            "FeaturedBundle": {"Bundles": [{
                "DataAssetID": "3941ad01-4e3b-46e0-ba3a-ab94f7c67f98", "Items": [{}, {}, {}],
                "TotalDiscountedCost": {"85ad13f7-3d1b-5128-9eb2-7cd8ee0b5741": 5350},
                "DurationRemainingInSeconds": 200_000
            }]},
            "SkinsPanelLayout": {
                "SingleItemOffersRemainingDurationInSeconds": 40_000,
                "SingleItemStoreOffers": [{
                    "Cost": {"85ad13f7-3d1b-5128-9eb2-7cd8ee0b5741": 875},
                    "Rewards": [{"ItemID": "bad7ae62-4f6a-8974-ab73-7ba6405ca898"}]
                }]
            }
        })),
        wallet: Some(json!({"Balances": {"85ad13f7-3d1b-5128-9eb2-7cd8ee0b5741": 1450}})),
        matches: std::iter::once(("demo-match".to_owned(), review_details(0)))
            .chain((0..20).map(|i| (format!("history-{i}"), review_details(i))))
            .collect(),
        content: json!({"Seasons": [
            {"ID": "ep-v26", "Name": "V26", "Type": "episode"},
            {"ID": "season-prev", "Name": "ACT 2", "Type": "act", "EndTime": "t1"},
            {"ID": "season-now", "Name": "ACT 3", "Type": "act", "StartTime": "t1", "IsActive": true}
        ]}),
        agents: AGENTS.iter().map(|(id, name)| ((*id).to_owned(), (*name).to_owned())).collect(),
        weapons: HashMap::from([("weapon-vandal".to_owned(), "Vandal".to_owned())]),
        maps: HashMap::from([
            ("/game/maps/ascent/ascent".to_owned(), "Ascent".to_owned()),
            ("/game/maps/bonsai/bonsai".to_owned(), "Split".to_owned()),
            ("/game/maps/triad/triad".to_owned(), "Haven".to_owned()),
            ("/game/maps/duality/duality".to_owned(), "Bind".to_owned()),
            ("/game/maps/port/port".to_owned(), "Icebox".to_owned()),
        ]),
    }
}
