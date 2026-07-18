use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions (
            id              TEXT PRIMARY KEY,
            cli             TEXT NOT NULL,
            cli_display_name TEXT NOT NULL,
            directory       TEXT,
            pid             INTEGER,
            status          TEXT NOT NULL DEFAULT 'running',
            created_at      TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS token_usage (
            id              TEXT PRIMARY KEY,
            session_id      TEXT NOT NULL REFERENCES sessions(id),
            model           TEXT NOT NULL,
            input_tokens    INTEGER NOT NULL DEFAULT 0,
            output_tokens   INTEGER NOT NULL DEFAULT 0,
            cost_estimate   REAL NOT NULL DEFAULT 0.0,
            recorded_at     TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agents (
            id              TEXT PRIMARY KEY,
            display_name    TEXT NOT NULL,
            installed       INTEGER NOT NULL DEFAULT 0,
            enabled         INTEGER NOT NULL DEFAULT 1,
            status          TEXT NOT NULL DEFAULT 'not_installed',
            version         TEXT,
            install_path    TEXT,
            last_tested_at  TEXT,
            detected_at     TEXT
        );

        CREATE TABLE IF NOT EXISTS models (
            id              TEXT PRIMARY KEY,
            name            TEXT NOT NULL,
            alias           TEXT NOT NULL DEFAULT '',
            provider        TEXT NOT NULL,
            provider_name   TEXT NOT NULL,
            provider_icon   TEXT NOT NULL,
            api_base        TEXT NOT NULL,
            api_key         TEXT NOT NULL,
            protocol        TEXT NOT NULL DEFAULT 'openai',
            created_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS model_aliases (
            alias           TEXT PRIMARY KEY,
            model_id        TEXT NOT NULL REFERENCES models(id),
            description     TEXT NOT NULL DEFAULT '',
            created_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key             TEXT PRIMARY KEY,
            value           TEXT NOT NULL,
            updated_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS agent_models (
            agent_id        TEXT NOT NULL REFERENCES agents(id),
            model_id        TEXT NOT NULL REFERENCES models(id),
            created_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (agent_id, model_id)
        );

        CREATE TABLE IF NOT EXISTS messages (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id      TEXT NOT NULL,
            role            TEXT NOT NULL,
            content         TEXT NOT NULL,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )
    .expect("Failed to run migrations");

    conn.execute(
        "ALTER TABLE models ADD COLUMN context_window INTEGER NOT NULL DEFAULT 0",
        [],
    ).ok();

    conn.execute(
        "ALTER TABLE models ADD COLUMN support_reasoning INTEGER NOT NULL DEFAULT 0",
        [],
    ).ok();

    conn.execute(
        "ALTER TABLE models ADD COLUMN tags TEXT NOT NULL DEFAULT ''",
        [],
    ).ok();

    conn.execute(
        "ALTER TABLE agent_models ADD COLUMN use_proxy INTEGER NOT NULL DEFAULT 0",
        [],
    ).ok();

    conn.execute(
        "ALTER TABLE agent_models ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0",
        [],
    ).ok();
}
