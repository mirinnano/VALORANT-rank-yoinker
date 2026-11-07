//! 遭遇履歴の保存。保存するのはスナップショットの値だけで、隠された本名は持たない。

use crate::error::Result;
use crate::model::{MatchSnapshot, PlayerView, Relation};
use crate::review::MatchReview;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Encounter {
    pub display_name: String,
    pub agent: Option<String>,
    pub map: Option<String>,
    pub relation: String,
    pub rank: String,
    pub seen_at_ms: i64,
    pub times_met: u32,
}

/// 自己レビューの記録。`round` が `None` なら試合全体へのメモ。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewNote {
    pub round: Option<u32>,
    pub tags: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TagCount {
    pub tag: String,
    pub count: u32,
}

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// 試合全体のメモを表す `round` の値。
const WHOLE_MATCH: i64 = -1;

pub struct Store {
    db: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS encounters (
    puuid TEXT NOT NULL,
    match_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    agent TEXT,
    map TEXT,
    relation TEXT NOT NULL,
    rank TEXT NOT NULL,
    seen_at_ms INTEGER NOT NULL,
    PRIMARY KEY (puuid, match_id)
);
CREATE INDEX IF NOT EXISTS encounters_seen ON encounters (seen_at_ms DESC);
CREATE TABLE IF NOT EXISTS match_reviews (
    match_id TEXT PRIMARY KEY,
    review TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS review_notes (
    match_id TEXT NOT NULL,
    round INTEGER NOT NULL,
    tags TEXT NOT NULL,
    text TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (match_id, round)
);
";

fn relation_label(relation: Relation) -> Option<&'static str> {
    match relation {
        Relation::Ally => Some("ally"),
        Relation::Enemy => Some("enemy"),
        Relation::Own | Relation::Party => None,
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        // エンジンと画面が別の接続で同時に使うので、読み書きが互いを止めないようにする。
        db.busy_timeout(BUSY_TIMEOUT)?;
        db.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
    }

    /// 今回の試合を除いた遭遇回数。
    pub fn counts(&self, puuids: &[String], match_id: Option<&str>) -> Result<HashMap<String, u32>> {
        let mut query = self.db.prepare_cached(
            "SELECT COUNT(*) FROM encounters WHERE puuid = ?1 AND match_id IS NOT ?2",
        )?;
        let mut counts = HashMap::new();
        for puuid in puuids {
            let count: u32 = query.query_row(params![puuid, match_id], |row| row.get(0))?;
            if count > 0 {
                counts.insert(puuid.clone(), count);
            }
        }
        Ok(counts)
    }

    /// 試合の参加者を記録する。自分とパーティは記録しない。
    pub fn record(&mut self, snapshot: &MatchSnapshot, now_ms: i64) -> Result<()> {
        let Some(match_id) = snapshot.match_id.as_deref() else { return Ok(()) };
        let tx = self.db.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO encounters
                     (puuid, match_id, display_name, agent, map, relation, rank, seen_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (puuid, match_id) DO UPDATE SET
                     display_name = excluded.display_name,
                     agent = excluded.agent,
                     rank = excluded.rank",
            )?;
            for player in &snapshot.players {
                let PlayerView { puuid, display_name, agent, relation, rank, .. } = player;
                let Some(relation) = relation_label(*relation) else { continue };
                insert.execute(params![
                    puuid, match_id, display_name, agent, snapshot.map, relation, rank.name, now_ms
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 集計済みの振り返りを返す。試合結果は変わらないので、そのまま再利用できる。
    pub fn cached_review(&self, match_id: &str) -> Result<Option<serde_json::Value>> {
        let text: Option<String> = self
            .db
            .query_row("SELECT review FROM match_reviews WHERE match_id = ?1", [match_id], |row| row.get(0))
            .optional()?;
        text.map(|text| serde_json::from_str(&text).map_err(Into::into)).transpose()
    }

    pub fn cache_review(&self, match_id: &str, review: &MatchReview) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO match_reviews (match_id, review) VALUES (?1, ?2)",
            params![match_id, serde_json::to_string(review)?],
        )?;
        Ok(())
    }

    /// メモを保存する。タグも本文も空なら削除する。
    pub fn save_note(&self, match_id: &str, note: &ReviewNote, now_ms: i64) -> Result<()> {
        let round = note.round.map_or(WHOLE_MATCH, i64::from);
        if note.tags.is_empty() && note.text.trim().is_empty() {
            self.db.execute(
                "DELETE FROM review_notes WHERE match_id = ?1 AND round = ?2",
                params![match_id, round],
            )?;
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO review_notes (match_id, round, tags, text, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (match_id, round) DO UPDATE SET
                 tags = excluded.tags, text = excluded.text, updated_at_ms = excluded.updated_at_ms",
            params![match_id, round, serde_json::to_string(&note.tags)?, note.text, now_ms],
        )?;
        Ok(())
    }

    pub fn notes(&self, match_id: &str) -> Result<Vec<ReviewNote>> {
        let mut query = self.db.prepare_cached(
            "SELECT round, tags, text FROM review_notes WHERE match_id = ?1 ORDER BY round",
        )?;
        let rows = query.query_map([match_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut notes = Vec::new();
        for row in rows {
            let (round, tags, text) = row?;
            notes.push(ReviewNote {
                round: u32::try_from(round).ok(),
                tags: serde_json::from_str(&tags)?,
                text,
            });
        }
        Ok(notes)
    }

    /// メモのある試合の一覧。
    pub fn reviewed_matches(&self) -> Result<Vec<String>> {
        let mut query = self.db.prepare_cached("SELECT DISTINCT match_id FROM review_notes")?;
        let rows = query.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// 直近のメモで多いタグ。繰り返す課題を見つけるために使う。
    pub fn tag_trend(&self, recent_notes: u32) -> Result<Vec<TagCount>> {
        let mut query = self.db.prepare_cached(
            "SELECT tags FROM review_notes ORDER BY updated_at_ms DESC LIMIT ?1",
        )?;
        let rows = query.query_map([recent_notes], |row| row.get::<_, String>(0))?;
        let mut counts: HashMap<String, u32> = HashMap::new();
        for row in rows {
            let tags: Vec<String> = serde_json::from_str(&row?)?;
            for tag in tags {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut trend: Vec<TagCount> =
            counts.into_iter().map(|(tag, count)| TagCount { tag, count }).collect();
        trend.sort_by(|a, b| b.count.cmp(&a.count).then(a.tag.cmp(&b.tag)));
        Ok(trend)
    }

    /// 直近の遭遇を新しい順に返す。puuid は UI へ渡さない。
    pub fn recent(&self, limit: u32) -> Result<Vec<Encounter>> {
        let mut query = self.db.prepare_cached(
            "SELECT e.display_name, e.agent, e.map, e.relation, e.rank, e.seen_at_ms,
                    (SELECT COUNT(*) FROM encounters o WHERE o.puuid = e.puuid)
             FROM encounters e
             ORDER BY e.seen_at_ms DESC, e.rowid DESC
             LIMIT ?1",
        )?;
        let rows = query.query_map([limit], |row| {
            Ok(Encounter {
                display_name: row.get(0)?,
                agent: row.get(1)?,
                map: row.get(2)?,
                relation: row.get(3)?,
                rank: row.get(4)?,
                seen_at_ms: row.get(5)?,
                times_met: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameState;
    use crate::rank::view;

    fn player(puuid: &str, name: &str, relation: Relation) -> PlayerView {
        PlayerView {
            puuid: puuid.into(),
            key: 0,
            display_name: name.into(),
            name_hidden: false,
            relation,
            team: None,
            agent: None,
            agent_locked: true,
            level: None,
            party: None,
            rank: view(12, 0, 0),
            peak: view(12, 0, 0),
            peak_act: None,
            previous_rank: view(0, 0, 0),
            win_rate: None,
            games: 0,
            kd: None,
            headshot_percent: None,
            earned_rr: None,
            afk_penalty: None,
            last_active_ms: None,
            recent_rr: Vec::new(),
            loading: false,
            encounters: 0,
        }
    }

    fn snapshot(id: &str, players: Vec<PlayerView>) -> MatchSnapshot {
        MatchSnapshot {
            state: GameState::InGame,
            match_id: Some(id.into()),
            mode: None,
            map: Some("Ascent".into()),
            players,
            ally_average: None,
            enemy_average: None,
            profile: None,
            score: None,
            finished_match: None,
        }
    }

    fn note(round: Option<u32>, tags: &[&str], text: &str) -> ReviewNote {
        ReviewNote { round, tags: tags.iter().map(|t| t.to_string()).collect(), text: text.into() }
    }

    #[test]
    fn notes_round_trip_and_replace_by_round() {
        let store = Store::in_memory().unwrap();
        store.save_note("m", &note(None, &["判断"], "全体"), 1).unwrap();
        store.save_note("m", &note(Some(4), &["エイム"], "最初"), 2).unwrap();
        store.save_note("m", &note(Some(4), &["立ち位置"], "直した"), 3).unwrap();
        store.save_note("other", &note(None, &["連携"], "別の試合"), 4).unwrap();
        let notes = store.notes("m").unwrap();
        assert_eq!(notes, [note(None, &["判断"], "全体"), note(Some(4), &["立ち位置"], "直した")]);
    }

    #[test]
    fn review_cache_round_trips_and_misses_cleanly() {
        let store = Store::in_memory().unwrap();
        assert!(store.cached_review("m").unwrap().is_none());

        let line = crate::review::ScoreLine {
            label: "あなた".into(), agent: None, is_own: true, is_ally: true, rank: "Gold 1", tier: 12,
            acs: 200, kills: 1, deaths: 1, assists: 0, adr: 90,
            headshot_percent: None, first_kills: 0, first_deaths: 0,
        };
        let review = MatchReview {
            match_id: "m".into(), map: None, started_ms: 0, length_ms: 0, won: Some(true),
            own_score: 13, enemy_score: 5, own: line.clone(), team_acs: 0, team_adr: 0,
            lines: vec![line], rounds: Vec::new(), findings: Vec::new(),
        };
        store.cache_review("m", &review).unwrap();
        assert_eq!(store.cached_review("m").unwrap().unwrap()["own_score"], 13);
    }

    #[test]
    fn empty_note_removes_the_record() {
        let store = Store::in_memory().unwrap();
        store.save_note("m", &note(Some(1), &["判断"], ""), 1).unwrap();
        store.save_note("m", &note(Some(1), &[], "  "), 2).unwrap();
        assert!(store.notes("m").unwrap().is_empty());
        assert!(store.reviewed_matches().unwrap().is_empty());
    }

    #[test]
    fn tag_trend_counts_recent_notes_only() {
        let store = Store::in_memory().unwrap();
        store.save_note("old", &note(None, &["古い"], ""), 1).unwrap();
        store.save_note("a", &note(None, &["判断", "エイム"], ""), 2).unwrap();
        store.save_note("b", &note(None, &["判断"], ""), 3).unwrap();
        let trend = store.tag_trend(2).unwrap();
        assert_eq!(trend[0], TagCount { tag: "判断".into(), count: 2 });
        assert_eq!(trend.len(), 2);
    }

    #[test]
    fn records_only_other_players() {
        let mut store = Store::in_memory().unwrap();
        let s = snapshot("m1", vec![
            player("me", "Me#1", Relation::Own),
            player("friend", "F#1", Relation::Party),
            player("x", "X#1", Relation::Enemy),
        ]);
        store.record(&s, 100).unwrap();
        let recent = store.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].display_name, "X#1");
    }

    #[test]
    fn counts_exclude_the_current_match() {
        let mut store = Store::in_memory().unwrap();
        store.record(&snapshot("m1", vec![player("x", "X#1", Relation::Ally)]), 1).unwrap();
        store.record(&snapshot("m2", vec![player("x", "X#1", Relation::Enemy)]), 2).unwrap();
        let ids = ["x".to_string()];
        assert_eq!(store.counts(&ids, Some("m2")).unwrap()["x"], 1);
        assert_eq!(store.counts(&ids, None).unwrap()["x"], 2);
    }

    #[test]
    fn repeated_record_of_a_match_is_idempotent() {
        let mut store = Store::in_memory().unwrap();
        let s = snapshot("m1", vec![player("x", "X#1", Relation::Enemy)]);
        store.record(&s, 1).unwrap();
        store.record(&s, 2).unwrap();
        assert_eq!(store.recent(10).unwrap().len(), 1);
        assert_eq!(store.recent(10).unwrap()[0].times_met, 1);
    }

    #[test]
    fn hidden_player_is_stored_under_the_display_name() {
        let mut store = Store::in_memory().unwrap();
        let mut hidden = player("x", "Jett", Relation::Enemy);
        hidden.name_hidden = true;
        store.record(&snapshot("m1", vec![hidden]), 1).unwrap();
        assert_eq!(store.recent(1).unwrap()[0].display_name, "Jett");
    }
}
