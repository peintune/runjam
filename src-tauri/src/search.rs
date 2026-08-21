use rusqlite::{Connection, Result, params};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use crate::rjlog;

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct SessionRecord {
    pub id: String,
    pub cli: String,
    pub cli_display_name: String,
    pub title: String,
    pub directory: String,
    pub model: Option<String>,
    pub status: String,
    pub pid: Option<i64>,
    pub pinned: i64,
    pub archived: i64,
    pub created_at: String,
    pub last_active_at: Option<String>,
    pub acp_session_id: String,
    /// Total character count of the session's messages, persisted so the
    /// sidebar can show every session's context size without loading all
    /// messages into the frontend.
    pub context_chars: i64,
}

fn db_path() -> PathBuf {
    // Database lives in ~/.runjam/runjam.db (unified user data dir).
    let base = directories::UserDirs::new()
        .map(|d| d.home_dir().join(".runjam"))
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&base).ok();
    base.join("runjam.db")
}

fn get_conn() -> Result<Connection> {
    let conn = Connection::open(db_path())?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    // 删除会话/保存消息时，正在流式写入的 agent 进程可能持有写锁；
    // 默认 busy_timeout=0 会让 DELETE 立即报 "database is locked" 而失败，
    // 表现为"删除会话不成功"。设置等待时间让 SQLite 等锁释放。
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(conn)
}

pub fn init_db() {
    if let Ok(conn) = get_conn() {
        // Table creation — each group is independent so a failure in one
        // (e.g. ALTER TABLE on an existing column) doesn't block the rest.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                cli TEXT NOT NULL,
                cli_display_name TEXT NOT NULL,
                title TEXT,
                directory TEXT,
                status TEXT NOT NULL DEFAULT 'running',
                pid INTEGER,
                pinned INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
                last_active_at TEXT
            );"
        ).ok();
        // Schema migrations (ignore errors if column already exists)
        conn.execute("ALTER TABLE sessions ADD COLUMN title TEXT", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN directory TEXT", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN pinned INTEGER DEFAULT 0", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN archived INTEGER DEFAULT 0", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN acp_session_id TEXT DEFAULT ''", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN model TEXT DEFAULT ''", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN last_active_at TEXT", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN context_chars INTEGER DEFAULT 0", []).ok();
        // Backfill last_active_at = created_at for rows created before the
        // column existed, so the sidebar time display stays sensible.
        conn.execute(
            "UPDATE sessions SET last_active_at = created_at WHERE last_active_at IS NULL",
            [],
        ).ok();
        // Messages table
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);"
        ).ok();
        // Backfill context_chars from already-saved messages so existing
        // sessions show a context size before their next message is saved.
        conn.execute(
            "UPDATE sessions SET context_chars = COALESCE(
                (SELECT SUM(LENGTH(content)) FROM messages WHERE session_id = sessions.id), 0
             ) WHERE context_chars IS NULL OR context_chars = 0",
            [],
        ).ok();
        // FTS5 virtual table + triggers (separate batch so earlier failures
        // don't prevent search from working)
        let fts_result = conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                session_id, role, content,
                content=messages, content_rowid=id
            );
            CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                INSERT INTO messages_fts(rowid, session_id, role, content)
                VALUES (new.id, new.session_id, new.role, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, session_id, role, content)
                VALUES ('delete', old.id, old.session_id, old.role, old.content);
            END;"
        );
        if let Err(e) = fts_result {
            rjlog!("[DB ERROR] FTS5 init failed: {}", e);
        }
    }
}

/// Saves a message and returns the session's updated context_chars so the
/// caller can refresh the sidebar without re-querying the whole session.
pub fn save_message(session_id: &str, role: &str, content: &str) -> i64 {
    let Ok(conn) = get_conn() else { return 0 };
    save_message_on(&conn, session_id, role, content)
}

pub(crate) fn save_message_on(conn: &Connection, session_id: &str, role: &str, content: &str) -> i64 {
    // Insert the message, then bump the session's last_active_at so the
    // sidebar time and ordering reflect the freshest activity (covers both
    // user-sent and agent-responded messages, since both go through here).
    let _ = conn.execute(
        "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
        params![session_id, role, content],
    );
    let _ = conn.execute(
        "UPDATE sessions SET last_active_at = datetime('now','localtime') WHERE id = ?1",
        params![session_id],
    );
    refresh_context_chars_on(conn, session_id)
}

/// Recomputes a session's `context_chars` from its stored messages and
/// returns the new value. The SUM is recomputed (rather than incrementally
/// added) so the stored value stays correct even if messages are later
/// re-saved or deleted.
pub(crate) fn refresh_context_chars_on(conn: &Connection, session_id: &str) -> i64 {
    let _ = conn.execute(
        "UPDATE sessions SET context_chars = COALESCE(
            (SELECT SUM(LENGTH(content)) FROM messages WHERE session_id = ?1), 0
         ) WHERE id = ?1",
        params![session_id],
    );
    conn.query_row(
        "SELECT COALESCE(context_chars, 0) FROM sessions WHERE id = ?1",
        params![session_id],
        |row| row.get(0),
    ).unwrap_or(0)
}

pub fn search_messages(query: &str, limit: usize) -> Vec<SearchResult> {
    let conn = match get_conn() { Ok(c) => c, Err(_) => return vec![] };
    // Wrap the query in double quotes so FTS5 treats it as a string literal
    // (prevents *, :, "" etc. from being interpreted as FTS5 syntax).
    let safe_query = format!("\"{}\"", query.replace("\"", "\"\""));
    let mut stmt = match conn.prepare(
        "SELECT m.session_id, m.role, m.content, m.created_at
         FROM messages_fts fts
         JOIN messages m ON m.id = fts.rowid
         WHERE messages_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2"
    ) { Ok(s) => s, Err(e) => { rjlog!("[SEARCH ERROR] prepare failed: {}", e); return vec![]; } };

    let results = stmt.query_map(params![safe_query, limit as i64], |row| {
        Ok(SearchResult {
            session_id: row.get(0)?,
            role: row.get(1)?,
            content: row.get(2)?,
            created_at: row.get(3)?,
        })
    });

    match results {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(e) => { rjlog!("[SEARCH ERROR] query failed: {}", e); vec![] }
    }
}

pub fn get_messages_by_session(session_id: &str) -> Vec<SearchResult> {
    let conn = match get_conn() { Ok(c) => c, Err(_) => return vec![] };
    let mut stmt = match conn.prepare(
        "SELECT session_id, role, content, created_at
         FROM messages
         WHERE session_id = ?1
         ORDER BY created_at ASC"
    ) { Ok(s) => s, Err(_) => return vec![] };

    let results = stmt.query_map(params![session_id], |row| {
        Ok(SearchResult {
            session_id: row.get(0)?,
            role: row.get(1)?,
            content: row.get(2)?,
            created_at: row.get(3)?,
        })
    });

    match results {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    }
}

pub fn save_session(
    id: &str,
    cli: &str,
    cli_display_name: &str,
    title: &str,
    directory: &str,
    status: &str,
    pid: Option<i64>,
    pinned: i64,
    archived: i64,
    acp_session_id: &str,
) {
    if let Ok(conn) = get_conn() {
        save_session_on(&conn, id, cli, cli_display_name, title, directory, status, pid, pinned, archived, acp_session_id);
    }
}

pub(crate) fn save_session_on(
    conn: &Connection,
    id: &str,
    cli: &str,
    cli_display_name: &str,
    title: &str,
    directory: &str,
    status: &str,
    pid: Option<i64>,
    pinned: i64,
    archived: i64,
    acp_session_id: &str,
) {
    // A brand-new session's last_active_at starts equal to its created_at so
    // the sidebar sorts it at creation time. INSERT OR REPLACE also covers
    // later updates from save_session — for those we keep the existing
    // last_active_at (the COALESCE picks up the row's prior value).
    let result = conn.execute(
        "INSERT OR REPLACE INTO sessions (id, cli, cli_display_name, title, directory, status, pid, pinned, created_at, last_active_at, archived, acp_session_id, context_chars)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 COALESCE((SELECT created_at FROM sessions WHERE id = ?1), datetime('now', 'localtime')),
                 COALESCE((SELECT last_active_at FROM sessions WHERE id = ?1), datetime('now', 'localtime')),
                 ?9, ?10,
                 COALESCE((SELECT context_chars FROM sessions WHERE id = ?1), 0))",
        params![id, cli, cli_display_name, title, directory, status, pid, pinned, archived, acp_session_id],
    );
    if let Err(e) = result {
        rjlog!("[DB ERROR] save_session failed: {}", e);
    }
}

pub fn get_sessions() -> Vec<SessionRecord> {
    let conn = match get_conn() { Ok(c) => c, Err(e) => { rjlog!("[DB ERROR] get_conn failed: {}", e); return vec![]; } };
    get_sessions_on(&conn)
}

pub(crate) fn get_sessions_on(conn: &Connection) -> Vec<SessionRecord> {
    let mut stmt = match conn.prepare(
        "SELECT id, cli, cli_display_name, title, directory, model, status, pid, pinned, created_at, last_active_at, archived, acp_session_id, context_chars
         FROM sessions
         ORDER BY pinned DESC, COALESCE(last_active_at, created_at) DESC"
    ) { Ok(s) => s, Err(e) => { rjlog!("[DB ERROR] prepare get_sessions failed: {}", e); return vec![]; } };

    let results = stmt.query_map([], |row| {
        Ok(SessionRecord {
            id: row.get(0)?,
            cli: row.get(1)?,
            cli_display_name: row.get(2)?,
            title: row.get(3)?,
            directory: row.get(4)?,
            model: row.get(5)?,
            status: row.get(6)?,
            pid: row.get(7)?,
            pinned: row.get(8)?,
            created_at: row.get(9)?,
            last_active_at: row.get(10)?,
            archived: row.get(11)?,
            acp_session_id: row.get(12)?,
            context_chars: row.get(13)?,
        })
    });

    match results {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    }
}

pub fn touch_session(id: &str) {
    if let Ok(conn) = get_conn() {
        touch_session_on(&conn, id);
    }
}

pub(crate) fn touch_session_on(conn: &Connection, id: &str) {
    let _ = conn.execute(
        "UPDATE sessions SET last_active_at = datetime('now', 'localtime') WHERE id = ?1",
        params![id],
    );
}

pub fn set_session_archived(id: &str, archived: bool) {
    if let Ok(conn) = get_conn() {
        conn.execute(
            "UPDATE sessions SET archived = ?1 WHERE id = ?2",
            params![archived as i64, id],
        ).ok();
    }
}

pub fn set_session_model(id: &str, model: &str) {
    if let Ok(conn) = get_conn() {
        conn.execute(
            "UPDATE sessions SET model = ?1 WHERE id = ?2",
            params![model, id],
        ).ok();
    }
}

pub fn delete_archived_sessions() {
    if let Ok(conn) = get_conn() {
        conn.execute("DELETE FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE archived = 1)", []).ok();
        conn.execute("DELETE FROM token_usage WHERE session_id IN (SELECT id FROM sessions WHERE archived = 1)", []).ok();
        if conn.execute("DELETE FROM sessions WHERE archived = 1", []).is_err() {
            conn.execute_batch("PRAGMA foreign_keys=OFF").ok();
            conn.execute("DELETE FROM sessions WHERE archived = 1", []).ok();
            conn.execute_batch("PRAGMA foreign_keys=ON").ok();
        }
    }
}

pub fn update_session_title(id: &str, title: &str) {
    if let Ok(conn) = get_conn() {
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE id = ?2",
            params![title, id],
        ).ok();
    }
}

pub fn delete_session(id: &str) -> Result<()> {
    let conn = get_conn()?;
    delete_session_on(&conn, id)
}

/// Deletes a session and its related rows on a given connection (used by
/// tests with an in-memory DB; the public wrapper opens the real DB).
pub(crate) fn delete_session_on(conn: &Connection, id: &str) -> Result<()> {
    // Delete related data (best-effort, don't fail if any step errors)
    let _ = conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id]);
    let _ = conn.execute("DELETE FROM token_usage WHERE session_id = ?1", params![id]);

    // Try to delete the session. If FOREIGN KEY still blocks us (a table we
    // don't know about references sessions), temporarily disable FK
    // enforcement so the session row is always removed.
    if let Err(_) = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id]) {
        conn.execute_batch("PRAGMA foreign_keys=OFF")?;
        let result = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id]);
        conn.execute_batch("PRAGMA foreign_keys=ON")?;
        if let Err(e) = result {
            eprintln!("[delete_session] ERROR deleting session {}: {}", id, e);
            return Err(e);
        }
    }

    // Force WAL checkpoint so data survives app restart (no-op on in-memory)
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
    eprintln!("[delete_session] Deleted session: {}", id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Open an in-memory database with the same schema `init_db` produces.
    /// Tests use this to avoid touching the real `~/.runjam/runjam.db`.
    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Schema mirror of `init_db` (the relevant subset). Kept in sync
        // manually — both live in this file, so a divergence is caught by
        // tests. The `last_active_at` column is the one under test.
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                cli TEXT NOT NULL,
                cli_display_name TEXT NOT NULL,
                title TEXT,
                directory TEXT,
                status TEXT NOT NULL DEFAULT 'running',
                pid INTEGER,
                pinned INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
                last_active_at TEXT,
                archived INTEGER DEFAULT 0,
                acp_session_id TEXT DEFAULT '',
                model TEXT DEFAULT '',
                context_chars INTEGER DEFAULT 0
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );"
        ).unwrap();
        conn
    }

    #[test]
    fn new_session_gets_last_active_at_equal_to_created_at() {
        let conn = open_test_db();
        save_session_on(
            &conn,
            "s1", "claude", "Claude", "title", "/tmp", "running",
            None, 0, 0, "",
        );
        let rec = get_one_session_on(&conn, "s1").unwrap();
        assert_eq!(rec.last_active_at.as_deref(), Some(rec.created_at.as_str()));
    }

    #[test]
    fn touch_session_updates_last_active_at() {
        let conn = open_test_db();
        save_session_on(
            &conn, "s1", "claude", "Claude", "title", "/tmp", "running",
            None, 0, 0, "",
        );
        let before = get_one_session_on(&conn, "s1").unwrap().last_active_at.unwrap();
        // Sleep past the 1-second granularity of `datetime('now')` so the
        // timestamp actually changes.
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        touch_session_on(&conn, "s1");
        let after = get_one_session_on(&conn, "s1").unwrap().last_active_at.unwrap();
        assert_ne!(before, after, "touch_session should change last_active_at");
    }

    #[test]
    fn save_message_also_touches_session_last_active_at() {
        let conn = open_test_db();
        save_session_on(
            &conn, "s1", "claude", "Claude", "title", "/tmp", "running",
            None, 0, 0, "",
        );
        let before = get_one_session_on(&conn, "s1").unwrap().last_active_at.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        save_message_on(&conn, "s1", "user", "hi");
        let after = get_one_session_on(&conn, "s1").unwrap().last_active_at.unwrap();
        assert_ne!(before, after, "save_message should also touch the session");
    }

    #[test]
    fn save_message_updates_context_chars() {
        let conn = open_test_db();
        save_session_on(
            &conn, "s1", "claude", "Claude", "title", "/tmp", "running",
            None, 0, 0, "",
        );
        // No messages yet → context_chars is 0.
        assert_eq!(get_sessions_on(&conn)[0].context_chars, 0);
        // Each save returns the recomputed total char count.
        assert_eq!(save_message_on(&conn, "s1", "user", "hello"), 5);
        assert_eq!(save_message_on(&conn, "s1", "agent", "world!"), 11);
        assert_eq!(get_sessions_on(&conn)[0].context_chars, 11);
    }

    #[test]
    fn get_sessions_orders_by_last_active_at_desc() {
        let conn = open_test_db();
        save_session_on(&conn, "old", "claude", "C", "t", "/tmp", "running", None, 0, 0, "");
        std::thread::sleep(std::time::Duration::from_millis(1_100));
        save_session_on(&conn, "new", "claude", "C", "t", "/tmp", "running", None, 0, 0, "");
        // Order it so 'new' ends up on top by touching it last.
        touch_session_on(&conn, "new");
        let ordered = get_sessions_on(&conn);
        assert_eq!(ordered[0].id, "new");
        assert_eq!(ordered[1].id, "old");
    }

    // ── test-only read helpers ────────────────────────────────────

    fn get_one_session_on(conn: &Connection, id: &str) -> Option<TestSession> {
        conn.query_row(
            "SELECT id, created_at, last_active_at FROM sessions WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok(TestSession {
                id: row.get(0)?,
                created_at: row.get(1)?,
                last_active_at: row.get(2)?,
            }),
        ).ok()
    }

    #[derive(Debug)]
    struct TestSession {
        id: String,
        created_at: String,
        last_active_at: Option<String>,
    }
}
