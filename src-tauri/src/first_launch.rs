//! App-wide defaults are seeded once, before either webview starts.
use rusqlite::{Connection, OptionalExtension};
use std::path::PathBuf;

const MARKER: &str = "first_launch_defaults_v1";
pub const AGENTS: [&str; 3] = ["claude-code", "gemini-cli", "codex-cli"];

pub fn initialize(
    conn: &Connection,
    new_install: bool,
    paths: &[(&str, PathBuf)],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let initialized: Option<String> = tx
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [MARKER],
            |r| r.get(0),
        )
        .optional()?;
    if initialized.is_some() {
        return Ok(());
    }
    if new_install {
        for agent in AGENTS {
            let native = paths
                .iter()
                .find(|(id, _)| *id == agent)
                .map(|(_, path)| {
                    path.is_file()
                        && crate::agent_launch::read_native_launch_config(agent, path).is_ok()
                        && std::fs::read_to_string(path)
                            .map(|raw| !crate::models_config::config_is_overridden(agent, &raw))
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            for (key, value) in [
                (
                    format!("agent_config_mode_{agent}"),
                    if native { "native" } else { "managed" },
                ),
                (format!("agent_permission_{agent}"), "full_access"),
            ] {
                tx.execute(
                    "INSERT OR IGNORE INTO app_settings(key,value) VALUES(?1,?2)",
                    (key, value),
                )?;
            }
        }
    }
    // An existing installation gets a marker, not new permissions or modes.
    tx.execute(
        "INSERT INTO app_settings(key,value) VALUES(?1,?2)",
        (MARKER, if new_install { "fresh" } else { "existing" }),
    )?;
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT DEFAULT CURRENT_TIMESTAMP);").unwrap();
        conn
    }

    fn value(conn: &rusqlite::Connection, key: &str) -> String {
        conn.query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn fresh_install_prefers_readable_native_configs_and_full_access_for_all_agents() {
        let root =
            std::env::temp_dir().join(format!("runjam-first-launch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let paths: Vec<_> = ["claude-code", "gemini-cli", "codex-cli"]
            .into_iter()
            .map(|id| {
                let file = root.join(id);
                std::fs::write(
                    &file,
                    if id == "codex-cli" {
                        "model = 'native-model'"
                    } else {
                        "{}"
                    },
                )
                .unwrap();
                (id, file)
            })
            .collect();
        let conn = db();
        initialize(&conn, true, &paths).unwrap();
        for (id, _) in &paths {
            assert_eq!(value(&conn, &format!("agent_config_mode_{id}")), "native");
            assert_eq!(
                value(&conn, &format!("agent_permission_{id}")),
                "full_access"
            );
        }
        // A later launch must not reapply defaults over an explicit choice.
        conn.execute(
            "UPDATE app_settings SET value='ask_approval' WHERE key='agent_permission_codex-cli'",
            [],
        )
        .unwrap();
        initialize(&conn, true, &paths).unwrap();
        assert_eq!(value(&conn, "agent_permission_codex-cli"), "ask_approval");
        for (_, path) in &paths {
            assert!(path.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_install_without_new_marker_is_not_reclassified_as_first_launch() {
        let conn = db();
        conn.execute("INSERT INTO app_settings(key,value) VALUES('agent_permission_claude-code','read_only')", []).unwrap();
        initialize(&conn, false, &[]).unwrap();
        assert_eq!(value(&conn, "agent_permission_claude-code"), "read_only");
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM app_settings WHERE key LIKE 'agent_permission_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn missing_or_invalid_native_config_uses_managed_and_preserves_preset_choices() {
        let root = std::env::temp_dir().join(format!(
            "runjam-first-launch-invalid-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let invalid = root.join("invalid.json");
        std::fs::write(&invalid, "not json").unwrap();
        let conn = db();
        conn.execute(
            "INSERT INTO app_settings(key,value) VALUES('agent_permission_codex-cli','read_only')",
            [],
        )
        .unwrap();
        initialize(
            &conn,
            true,
            &[
                ("claude-code", invalid),
                ("gemini-cli", root.join("missing")),
            ],
        )
        .unwrap();
        assert_eq!(value(&conn, "agent_config_mode_claude-code"), "managed");
        assert_eq!(value(&conn, "agent_config_mode_gemini-cli"), "managed");
        assert_eq!(value(&conn, "agent_permission_codex-cli"), "read_only");
        std::fs::remove_dir_all(root).unwrap();
    }
}
