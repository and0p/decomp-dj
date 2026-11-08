//! Parity results for comparison cores, kept in the local database (docs/gui.md#parity).
//!
//! Frames are the tests. A frame that passed adds the code it ran to its core and build's
//! "passed" flags; a frame that failed is kept as a failure, with how to reproduce it (the
//! build, where the comparison core was synced, the failing frame) and the code it ran.
//! Code has parity when frames that passed ran it and no failing frame did.

use anyhow::Result;
use rusqlite::{params, Connection};

/// The coverage region parity is kept for.
pub const REGION: &str = "PRG";

/// A frame a comparison core failed on.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// Row id in the database (negative when kept only in memory).
    pub id: i64,
    /// The `[[compare]]` entry's name.
    pub core: String,
    /// sha256 of the comparison core's library when it failed.
    pub build: String,
    /// Where the comparison core was synced: (segment, frame). Its run started there.
    pub synced: (String, u64),
    pub segment: String,
    pub frame: u64,
    pub reason: String,
    /// Unix seconds.
    pub created: i64,
    /// The connector's coverage flags for the failing frame alone (empty for a failed sync).
    pub flags: Vec<u8>,
}

/// The flags of every frame that passed with `core` at `build`, OR'd together.
pub fn passed(conn: &Connection, core: &str, build: &str) -> Result<Vec<u8>> {
    let mut st = conn.prepare("SELECT flags FROM parity_passed WHERE core = ?1 AND build = ?2 AND region = ?3")?;
    let mut rows = st.query(params![core, build, REGION])?;
    Ok(match rows.next()? {
        Some(r) => r.get(0)?,
        None => vec![],
    })
}

/// Replaces the passed flags of `core` at `build`.
pub fn set_passed(conn: &Connection, core: &str, build: &str, flags: &[u8]) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO parity_passed(core, build, region, flags) VALUES (?1, ?2, ?3, ?4)", params![core, build, REGION, flags])?;
    Ok(())
}

/// Every failure kept for `core`, whatever the build, oldest first.
pub fn failures(conn: &Connection, core: &str) -> Result<Vec<Failure>> {
    let mut st = conn.prepare(
        "SELECT id, core, build, synced_segment, synced_frame, segment, frame, reason, created, flags FROM parity_failures WHERE core = ?1 AND region = ?2 ORDER BY id",
    )?;
    let rows = st.query_map(params![core, REGION], |r| {
        Ok(Failure {
            id: r.get(0)?,
            core: r.get(1)?,
            build: r.get(2)?,
            synced: (r.get(3)?, r.get::<_, i64>(4)? as u64),
            segment: r.get(5)?,
            frame: r.get::<_, i64>(6)? as u64,
            reason: r.get(7)?,
            created: r.get(8)?,
            flags: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Keeps a failure; returns its id.
pub fn add_failure(conn: &Connection, f: &Failure) -> Result<i64> {
    conn.execute(
        "INSERT INTO parity_failures(core, build, synced_segment, synced_frame, segment, frame, reason, created, region, flags) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![f.core, f.build, f.synced.0, f.synced.1 as i64, f.segment, f.frame as i64, f.reason, f.created, REGION, f.flags],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_failure(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM parity_failures WHERE id = ?1", [id])?;
    Ok(())
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(crate::cache::SCHEMA).unwrap();
        c
    }

    #[test]
    fn passed_flags_are_kept_per_core_and_build() {
        let c = db();
        assert!(passed(&c, "a", "b1").unwrap().is_empty());
        set_passed(&c, "a", "b1", &[1, 0, 1]).unwrap();
        set_passed(&c, "a", "b1", &[1, 1, 1]).unwrap();
        set_passed(&c, "a", "b2", &[0, 0, 1]).unwrap();
        assert_eq!(passed(&c, "a", "b1").unwrap(), vec![1, 1, 1]);
        assert_eq!(passed(&c, "a", "b2").unwrap(), vec![0, 0, 1]);
        assert!(passed(&c, "other", "b1").unwrap().is_empty());
    }

    #[test]
    fn failures_round_trip_and_delete() {
        let c = db();
        let f = Failure { id: 0, core: "a".into(), build: "b1".into(), synced: ("s".into(), 3), segment: "t".into(), frame: 9, reason: "RAM 0x02".into(), created: 5, flags: vec![0, 1] };
        let id = add_failure(&c, &f).unwrap();
        add_failure(&c, &Failure { core: "other".into(), ..f.clone() }).unwrap();
        let got = failures(&c, "a").unwrap();
        assert_eq!(got, vec![Failure { id, ..f }]);
        delete_failure(&c, id).unwrap();
        assert!(failures(&c, "a").unwrap().is_empty());
        assert_eq!(failures(&c, "other").unwrap().len(), 1);
    }
}
