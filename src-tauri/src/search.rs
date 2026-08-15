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
    pub acp_session_id: String,
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
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );"
        ).ok();
        // Schema migrations (ignore errors if column already exists)
        conn.execute("ALTER TABLE sessions ADD COLUMN title TEXT", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN directory TEXT", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN pinned INTEGER DEFAULT 0", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN archived INTEGER DEFAULT 0", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN acp_session_id TEXT DEFAULT ''", []).ok();
        conn.execute("ALTER TABLE sessions ADD COLUMN model TEXT DEFAULT ''", []).ok();
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

pub fn save_message(session_id: &str, role: &str, content: &str) {
    if let Ok(conn) = get_conn() {
        conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
            params![session_id, role, content],
        ).ok();
    }
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
        match conn.execute(
            "INSERT OR REPLACE INTO sessions (id, cli, cli_display_name, title, directory, status, pid, pinned, created_at, archived, acp_session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now', 'localtime'), ?9, ?10)",
            params![id, cli, cli_display_name, title, directory, status, pid, pinned, archived, acp_session_id],
        ) {
            Ok(_) => {},
            Err(e) => rjlog!("[DB ERROR] save_session failed: {}", e),
        }
    }
}

pub fn get_sessions() -> Vec<SessionRecord> {
    let conn = match get_conn() { Ok(c) => c, Err(e) => { rjlog!("[DB ERROR] get_conn failed: {}", e); return vec![]; } };
    let mut stmt = match conn.prepare(
        "SELECT id, cli, cli_display_name, title, directory, model, status, pid, pinned, created_at, archived, acp_session_id
         FROM sessions
         ORDER BY pinned DESC, created_at DESC"
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
            archived: row.get(10)?,
            acp_session_id: row.get(11)?,
        })
    });

    match results {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    }
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

    // Delete related data (best-effort, don't fail if any step errors)
    let _ = conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id]);
    let _ = conn.execute("DELETE FROM token_usage WHERE session_id = ?1", params![id]);

    // Try to delete the session. If FOREIGN KEY still blocks us, temporarily
    // disable FK enforcement so the session row is always removed.
    if let Err(_) = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id]) {
        conn.execute_batch("PRAGMA foreign_keys=OFF")?;
        let result = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id]);
        conn.execute_batch("PRAGMA foreign_keys=ON")?;
        if let Err(e) = result {
            eprintln!("[delete_session] ERROR deleting session {}: {}", id, e);
            return Err(e);
        }
    }

    // Force WAL checkpoint so data survives app restart
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
    eprintln!("[delete_session] Deleted session: {}", id);
    Ok(())
}
