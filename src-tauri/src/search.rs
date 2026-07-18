use rusqlite::{Connection, Result, params};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

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
    pub status: String,
    pub pid: Option<i64>,
    pub pinned: i64,
    pub archived: i64,
    pub created_at: String,
}

fn db_path() -> PathBuf {
    let base = directories::ProjectDirs::from("com", "runjam", "RunJam")
        .map(|d| d.data_local_dir().to_path_buf())
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
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ALTER TABLE sessions ADD COLUMN IF NOT EXISTS title TEXT;
            ALTER TABLE sessions ADD COLUMN IF NOT EXISTS directory TEXT;
            ALTER TABLE sessions ADD COLUMN IF NOT EXISTS pinned INTEGER DEFAULT 0;
            ALTER TABLE sessions ADD COLUMN IF NOT EXISTS archived INTEGER DEFAULT 0;
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
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
            END;",
        ).ok();
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
    let mut stmt = match conn.prepare(
        "SELECT m.session_id, m.role, m.content, m.created_at
         FROM messages_fts fts
         JOIN messages m ON m.id = fts.rowid
         WHERE messages_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2"
    ) { Ok(s) => s, Err(_) => return vec![] };

    let results = stmt.query_map(params![query, limit as i64], |row| {
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
) {
    if let Ok(conn) = get_conn() {
        conn.execute(
            "INSERT OR REPLACE INTO sessions (id, cli, cli_display_name, title, directory, status, pid, pinned, archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, cli, cli_display_name, title, directory, status, pid, pinned, archived],
        ).ok();
    }
}

pub fn get_sessions() -> Vec<SessionRecord> {
    let conn = match get_conn() { Ok(c) => c, Err(_) => return vec![] };
    let mut stmt = match conn.prepare(
        "SELECT id, cli, cli_display_name, title, directory, status, pid, pinned, archived, created_at
         FROM sessions
         ORDER BY pinned DESC, created_at DESC"
    ) { Ok(s) => s, Err(_) => return vec![] };

    let results = stmt.query_map([], |row| {
        Ok(SessionRecord {
            id: row.get(0)?,
            cli: row.get(1)?,
            cli_display_name: row.get(2)?,
            title: row.get(3)?,
            directory: row.get(4)?,
            status: row.get(5)?,
            pid: row.get(6)?,
            pinned: row.get(7)?,
            archived: row.get(8)?,
            created_at: row.get(9)?,
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

pub fn delete_archived_sessions() {
    if let Ok(conn) = get_conn() {
        conn.execute("DELETE FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE archived = 1)", []).ok();
        conn.execute("DELETE FROM sessions WHERE archived = 1", []).ok();
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

pub fn delete_session(id: &str) {
    if let Ok(conn) = get_conn() {
        conn.execute("DELETE FROM messages WHERE session_id = ?1", params![id]).ok();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id]).ok();
    }
}
