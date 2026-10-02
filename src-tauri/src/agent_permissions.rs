//! Default permission settings for the agent CLIs RunJam drives.
//!
//! THE PROBLEM: a freshly installed agent inside RunJam asks for approval on
//! ordinary work (running `curl`, writing a file, fetching a URL) and tells the
//! user to fix it by editing `~/.claude/settings.json`. That advice cannot work
//! here: RunJam points each CLI at its OWN config directory
//! (`agent_isolation`), so the user's file is never read and the instruction
//! turns into a dead end.
//!
//! THE FIX: RunJam pre-seeds the permission settings in the config file it owns,
//! so a first run is already unblocked. Two rules keep this from becoming a
//! hostile default:
//!
//!   1. MERGE, NEVER OVERWRITE — a key is written only when it is absent. A user
//!      who edits their permissions (in RunJam's config editor, or by hand) keeps
//!      whatever they chose; RunJam never fights them.
//!   2. GUARDRAILS, NOT A FREE-FOR-ALL — `permissions.deny` still blocks the
//!      irreversible commands (`sudo`, `rm -rf /`, `mkfs`, …) even though
//!      `Bash(*)` is allowed, and Gemini simply does not get its shell tool
//!      pre-approved. Users who want a real sandbox pick the read-only /
//!      ask-approval modes in the UI, which map onto each CLI's own switches
//!      (see `codex_permission_overrides`, `gemini_approval_mode`).
//!
//! codex is deliberately absent here: its approval policy and sandbox are passed
//! as ACP launch arguments (`acp_client`), which outrank `config.toml`. Writing
//! them into the file too would create a second, silently-ignored source of
//! truth.

use serde_json::{json, Map, Value};
use std::path::Path;

/// Tool rules Claude Code allows without prompting.
///
/// `permissions.allow` is evaluated BEFORE the `auto`-mode classifier, so a rule
/// listed here is what actually stops the prompts that tell the user to add
/// `Bash(curl:*)` themselves — the classifier never gets to see the call.
const CLAUDE_ALLOW: &[&str] = &[
    // Shell + files: the everyday loop of a coding agent.
    "Bash(*)",
    "Read(*)",
    "Edit(*)",
    "Write(*)",
    "NotebookEdit(*)",
    "Glob(*)",
    "Grep(*)",
    // Context gathering.
    "WebFetch(*)",
    "WebSearch(*)",
    "Task(*)",
    "TodoWrite(*)",
    // RunJam deploys skills into every session; the agent must be able to run
    // them without a prompt.
    "Skill(*)",
];

/// Guardrail rules. `permissions.deny` is checked first and wins over `allow`,
/// so `Bash(*)` above does not mean "anything goes". Deliberately narrow: this
/// stops the irreversible, it is not a sandbox.
const CLAUDE_DENY: &[&str] = &[
    "Bash(sudo:*)",
    "Bash(rm -rf /:*)",
    "Bash(rm -fr /:*)",
    "Bash(mkfs:*)",
    "Bash(dd:*)",
    "Bash(shutdown:*)",
    "Bash(reboot:*)",
    "Bash(halt:*)",
];

/// Gemini CLI core tools allowed without confirmation.
///
/// Deliberately EXCLUDES `run_shell_command`: Gemini has no deny-list, so
/// pre-approving its shell tool would approve every command with no guardrail at
/// all. Shell calls still work — they arrive as a permission request, which
/// RunJam answers automatically in `approve_for_me` / `full_access` and denies in
/// `read_only`.
const GEMINI_ALLOWED_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "replace",
    "read_many_files",
    "glob",
    "grep",
    "ls",
    "list_directory",
    "google_web_search",
    "web_fetch",
    "write_todos",
];

/// Gemini's `general.defaultApprovalMode` when the user has not set one.
///
/// `auto_edit` (not `default`) so file edits stop asking. `yolo` is NOT a valid
/// value for this setting — Gemini accepts it only as a launch flag, which is why
/// `gemini_approval_mode` exists.
const GEMINI_DEFAULT_APPROVAL_MODE: &str = "auto_edit";

/// The registry of defaults, as `(dotted path in the settings file, value)`.
///
/// Keyed by both the RunJam ids (`claude-code`) and the short CLI names used on
/// the spawn path (`claude`), which must resolve to the same entry.
fn permission_defaults(agent_id: &str) -> Option<Vec<(&'static str, Value)>> {
    match agent_id {
        "claude" | "claude-code" => Some(vec![
            ("permissions.allow", json!(CLAUDE_ALLOW)),
            ("permissions.deny", json!(CLAUDE_DENY)),
        ]),
        "gemini" | "gemini-cli" => Some(vec![
            (
                "general.defaultApprovalMode",
                json!(GEMINI_DEFAULT_APPROVAL_MODE),
            ),
            ("tools.allowed", json!(GEMINI_ALLOWED_TOOLS)),
        ]),
        _ => None,
    }
}

/// Fill in RunJam's default permission settings for `agent_id`, if absent.
///
/// Idempotent, and cheap enough to call before every session start: the file is
/// only rewritten when something was actually added.
pub fn ensure_default_permissions(agent_id: &str) -> Result<(), String> {
    let Some(defaults) = permission_defaults(agent_id) else {
        return Ok(());
    };
    let path = crate::agent_isolation::isolated_config_file(agent_id)
        .ok_or_else(|| format!("no isolated config file for {}", agent_id))?;
    let changed = seed(&path, &defaults)?;
    if changed {
        crate::rjlog!(
            "[PERMS] Seeded {} default permission settings in {}",
            agent_id,
            path.display()
        );
    }
    Ok(())
}

/// Merge `defaults` into the JSON object at `path`, writing back only if a key
/// was actually missing. Returns whether anything changed.
fn seed(path: &Path, defaults: &[(&str, Value)]) -> Result<bool, String> {
    let mut settings = read_json_object(path);
    let mut changed = false;
    for (dotted, value) in defaults {
        if fill_if_absent(&mut settings, dotted, value.clone()) {
            changed = true;
        }
    }
    if changed {
        write_json_object(path, &settings)?;
    }
    Ok(changed)
}

/// Set `dotted` inside `map` only when it is absent, creating intermediate
/// objects as needed. Returns whether it was written.
///
/// An intermediate segment holding a non-object (a hand-written string, say) is
/// treated as "the user set this" and left alone: guessing at the user's intent
/// to overwrite their file is worse than skipping the default.
fn fill_if_absent(map: &mut Map<String, Value>, dotted: &str, value: Value) -> bool {
    let segments: Vec<&str> = dotted.split('.').collect();
    fill_segments(map, &segments, value)
}

fn fill_segments(map: &mut Map<String, Value>, segments: &[&str], value: Value) -> bool {
    let Some((head, rest)) = segments.split_first() else {
        return false;
    };
    if rest.is_empty() {
        if map.contains_key(*head) {
            return false;
        }
        map.insert((*head).to_string(), value);
        return true;
    }
    let entry = map
        .entry((*head).to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    match entry {
        Value::Object(child) => fill_segments(child, rest, value),
        _ => false,
    }
}

/// Read a JSON object, treating "missing" and "unparseable" alike as empty.
///
/// An agent may never have written this file yet, and RunJam owns it outright
/// (see `agent_isolation`), so there is nothing of the user's to preserve from a
/// broken one.
fn read_json_object(path: &Path) -> Map<String, Value> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write_json_object(path: &Path, value: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| format!("Failed to serialize {}: {}", path.display(), e))?;
    std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

/// The (approval policy, sandbox mode) codex should run with for a RunJam
/// permission mode. Passed as `-c` launch overrides, which take precedence over
/// `config.toml`.
pub fn codex_permission_overrides(permission_mode: &str) -> (&'static str, &'static str) {
    match permission_mode {
        // Nothing may run without approval, and the workspace is not writable.
        "read_only" => ("untrusted", "read-only"),
        // No approvals and no sandbox — the user asked for it explicitly.
        "full_access" => ("never", "danger-full-access"),
        // ask_approval / approve_for_me: codex may ask, and RunJam answers —
        // by prompting the user in `ask_approval`, automatically in
        // `approve_for_me`. That round trip is also what surfaces the command
        // in the UI.
        _ => ("on-request", "workspace-write"),
    }
}

/// The `--approval-mode=` value for a RunJam permission mode.
///
/// Unlike the settings file (which accepts only default/auto_edit/plan), the
/// launch flag accepts `yolo`, so this is the only way `full_access` reaches
/// gemini.
pub fn gemini_approval_mode(permission_mode: &str) -> &'static str {
    match permission_mode {
        "read_only" => "plan",
        "ask_approval" => "default",
        "full_access" => "yolo",
        _ => "auto_edit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A settings file in a throwaway directory. Tests deliberately do NOT use
    /// `RUNJAM_HOME`: that variable is process-wide and `models_config`'s tests
    /// set it too, so two modules touching it in parallel would race. Going
    /// straight through `seed()` keeps these tests hermetic and parallel-safe.
    struct TempFile {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn temp_settings(tag: &str) -> TempFile {
        let dir = std::env::temp_dir().join(format!(
            "runjam_perms_{}_{}_{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        TempFile { dir, path }
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn seed_agent(path: &Path, agent_id: &str) -> Result<bool, String> {
        let defaults = permission_defaults(agent_id).expect("agent has defaults");
        seed(path, &defaults)
    }

    #[test]
    fn claude_defaults_allow_bash_and_write_guardrails() {
        let file = temp_settings("claude_defaults");
        assert!(seed_agent(&file.path, "claude-code").unwrap());

        let settings = read(&file.path);
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        assert!(allow.iter().any(|v| v == "Bash(*)"), "Bash must be allowed");
        assert!(
            allow.iter().any(|v| v == "WebFetch(*)"),
            "WebFetch must be allowed"
        );

        let deny = settings["permissions"]["deny"].as_array().unwrap();
        assert!(
            deny.iter().any(|v| v == "Bash(sudo:*)"),
            "sudo must be denied"
        );
        assert!(
            deny.iter().any(|v| v == "Bash(rm -rf /:*)"),
            "rm -rf / must be denied"
        );
    }

    #[test]
    fn an_existing_allow_list_is_never_overwritten() {
        // The user's own rules win even when narrower than ours — silently
        // widening someone's permissions would be the worst outcome.
        let file = temp_settings("claude_user_wins");
        std::fs::write(
            &file.path,
            json!({ "permissions": { "allow": ["Bash(ls:*)"] }, "model": "mine" }).to_string(),
        )
        .unwrap();

        assert!(seed_agent(&file.path, "claude-code").unwrap());

        let settings = read(&file.path);
        assert_eq!(settings["permissions"]["allow"], json!(["Bash(ls:*)"]));
        assert_eq!(
            settings["model"],
            json!("mine"),
            "unrelated keys must survive"
        );
        // Only the absent half is filled in.
        assert!(settings["permissions"]["deny"].as_array().is_some());
    }

    #[test]
    fn a_malformed_permissions_value_is_left_alone() {
        // Someone hand-edited this into a string. Overwriting it would discard
        // their intent; we skip instead and let the CLI report the problem.
        let file = temp_settings("claude_malformed");
        std::fs::write(&file.path, json!({ "permissions": "junk" }).to_string()).unwrap();

        assert!(!seed_agent(&file.path, "claude-code").unwrap());
        assert_eq!(read(&file.path), json!({ "permissions": "junk" }));
    }

    #[test]
    fn seeding_is_idempotent_and_rewrites_nothing_on_a_second_pass() {
        let file = temp_settings("claude_idempotent");
        assert!(seed_agent(&file.path, "claude-code").unwrap());
        let first = std::fs::read_to_string(&file.path).unwrap();

        assert!(
            !seed_agent(&file.path, "claude-code").unwrap(),
            "a second pass must be a no-op"
        );
        assert_eq!(std::fs::read_to_string(&file.path).unwrap(), first);
    }

    #[test]
    fn an_unrelated_agent_has_no_defaults() {
        // codex is intentionally absent (launch args own its policy), and an
        // unknown id must not invent a file to write into.
        assert!(permission_defaults("codex-cli").is_none());
        assert!(permission_defaults("gpt-5").is_none());
        assert!(permission_defaults("ollama").is_none());
    }

    #[test]
    fn gemini_gets_auto_edit_and_no_shell_pre_approval() {
        let file = temp_settings("gemini_defaults");
        seed_agent(&file.path, "gemini-cli").unwrap();

        let settings = read(&file.path);
        assert_eq!(
            settings["general"]["defaultApprovalMode"],
            json!("auto_edit")
        );
        let allowed = settings["tools"]["allowed"].as_array().unwrap();
        assert!(allowed.iter().any(|v| v == "read_file"));
        assert!(
            !allowed.iter().any(|v| v == "run_shell_command"),
            "gemini has no deny-list, so pre-approving its shell tool would remove every guardrail"
        );
    }

    #[test]
    fn short_and_long_ids_share_one_default_set() {
        // The spawn path uses "claude"/"gemini"; the settings UI uses
        // "claude-code"/"gemini-cli". A divergence would mean the defaults are
        // seeded for one and silently missing for the other.
        assert_eq!(
            permission_defaults("claude"),
            permission_defaults("claude-code")
        );
        assert_eq!(
            permission_defaults("gemini"),
            permission_defaults("gemini-cli")
        );

        // And the path they resolve to is the isolated file, never `~/.claude`.
        for id in ["claude", "claude-code", "gemini", "gemini-cli"] {
            let path = crate::agent_isolation::isolated_config_file(id).unwrap();
            assert!(
                path.starts_with(crate::agent_isolation::isolated_config_root()),
                "{} must be seeded inside RunJam's own dir, got {path:?}",
                id
            );
        }
    }

    #[test]
    fn permission_modes_map_onto_each_cli_switch() {
        assert_eq!(
            codex_permission_overrides("read_only"),
            ("untrusted", "read-only")
        );
        assert_eq!(
            codex_permission_overrides("ask_approval"),
            ("on-request", "workspace-write")
        );
        assert_eq!(
            codex_permission_overrides("approve_for_me"),
            ("on-request", "workspace-write")
        );
        assert_eq!(
            codex_permission_overrides("full_access"),
            ("never", "danger-full-access")
        );

        assert_eq!(gemini_approval_mode("read_only"), "plan");
        assert_eq!(gemini_approval_mode("ask_approval"), "default");
        assert_eq!(gemini_approval_mode("approve_for_me"), "auto_edit");
        assert_eq!(gemini_approval_mode("full_access"), "yolo");
        // An unknown mode falls back to the unblocked-but-guarded default rather
        // than silently inheriting something more permissive.
        assert_eq!(gemini_approval_mode("nonsense"), "auto_edit");
        assert_eq!(
            codex_permission_overrides("nonsense"),
            ("on-request", "workspace-write")
        );
    }
}
