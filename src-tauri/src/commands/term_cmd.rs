use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Cap for the pre-mount PTY output buffer. If a terminal is never mounted for
/// a long time (e.g. a spawn superseded by a rapid session switch), the buffer
/// must not grow without bound — beyond this we drop the OLDEST bytes.
const MAX_PENDING_BYTES: usize = 1024 * 1024;

struct TerminalSlot {
    writer: Box<dyn MasterPty + Send>,
    cwd: String,
    _handle: thread::JoinHandle<()>,
    /// PTY output produced BEFORE the frontend attached its data listener.
    /// The read thread buffers every byte here so a session switch that races
    /// the xterm mount (tab.term still null at saveDirectoryState time) doesn't
    /// lose history — the frontend drains this on mount and writes it back.
    pending: Arc<Mutex<Vec<u8>>>,
}

pub struct TerminalState {
    terminals: HashMap<u32, TerminalSlot>,
    next_id: u32,
}

impl TerminalState {
    pub fn new() -> Self {
        Self { terminals: HashMap::new(), next_id: 1 }
    }
}

/// How the terminal shell is launched.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ShellMode {
    /// Full interactive shell (zsh -i / bash -i): loads ~/.zshrc with all
    /// prompt plugins. Rich but every idle shell keeps polling prompt state
    /// (git status, precmd hooks) and burns CPU.
    Interactive,
    /// Lightweight shell (zsh -f / bash --noprofile --norc): no rc loading,
    /// ~0% idle CPU. PATH is injected from the user's interactive shell so
    /// homebrew/nvm etc. still resolve.
    Lightweight,
}

/// Keywords that indicate an expensive rc file — either prompt frameworks that
/// poll on every prompt (burning idle CPU) or heavy runtime loaders (nvm, conda,
/// pyenv, …) that spawn subprocesses and execute on every shell start (burning
/// CPU whenever a terminal opens). Pure function — unit-testable.
fn rc_content_is_heavy(content: &str) -> bool {
    const EXPENSIVE_MARKERS: [&str; 17] = [
        // Prompt frameworks / plugins — idle CPU
        "oh-my-zsh",
        "powerlevel10k",
        "p10k",
        "starship",
        "zsh-syntax-highlighting",
        "zsh-autosuggestions",
        "antigen",
        "zplug",
        "zinit",
        "sheldon",
        "fzf",
        // Version-manager / runtime loaders — spawn CPU on every terminal open
        "nvm.sh",
        "conda",
        "pyenv",
        "rvm",
        "fnm",
        "mise",
    ];
    let lines = content.lines().count();
    lines > 60 || EXPENSIVE_MARKERS.iter().any(|k| content.contains(k))
}

/// Decide the shell launch mode by inspecting the user's rc file.
/// Expensive rc (prompt frameworks or heavy loaders like nvm/conda) →
/// Lightweight (the CPU fix); plain/minimal config stays Interactive so the
/// user loses nothing.
pub fn detect_shell_mode(shell: &str) -> ShellMode {
    let home = env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return ShellMode::Interactive;
    }
    let rc_path = if shell.ends_with("zsh") {
        std::path::Path::new(&home).join(".zshrc")
    } else if shell.ends_with("bash") {
        std::path::Path::new(&home).join(".bashrc")
    } else {
        return ShellMode::Interactive; // e.g. cmd.exe / fish — leave alone
    };
    match std::fs::read_to_string(rc_path) {
        Ok(content) if rc_content_is_heavy(&content) => ShellMode::Lightweight,
        _ => ShellMode::Interactive,
    }
}

/// PATH from the user's interactive shell, probed ONCE and cached.
/// A Tauri GUI process inherits launchd's minimal PATH (no homebrew/nvm); a
/// lightweight shell skips the rc file that would normally restore it, so we
/// recover it by running `$SHELL -ic 'printf %s "$PATH"'` with a 2s timeout.
static USER_PATH: OnceLock<Option<String>> = OnceLock::new();

fn detect_user_path(shell: &str) -> Option<String> {
    USER_PATH
        .get_or_init(|| {
            // Probe can be slow (real-world rc with nvm/conda takes seconds on
            // first load) — give it a generous budget since it runs ONCE.
            let mut child = ProcessCommand::new(shell)
                .arg("-ic")
                .arg("printf %s \"$PATH\"")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let deadline = Instant::now() + Duration::from_secs(12);
            let output = loop {
                match child.try_wait() {
                    Ok(Some(_)) => break child.wait_with_output().ok(),
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    _ => {
                        // Timed out — kill the probe and fall back to no PATH
                        // override (shell still starts with the default PATH).
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                }
            };
            output.and_then(|o| String::from_utf8(o.stdout).ok()).filter(|s| !s.is_empty())
        })
        .clone()
}

/// Warm the cached user PATH in the background so the first terminal spawn
/// doesn't block on the slow interactive-shell probe. Call once at app setup.
pub fn prefetch_user_path() {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let _ = detect_user_path(&shell);
}

#[tauri::command]
pub fn spawn_terminal(
    state: tauri::State<'_, Mutex<TerminalState>>,
    app: AppHandle,
    cwd: Option<String>,
) -> Result<u32, String> {
    let pty_system = native_pty_system();
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let pair = pty_system.openpty(size).map_err(|e| e.to_string())?;

    let shell: String = if cfg!(target_os = "windows") {
        "cmd.exe".to_string()
    } else {
        env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
    };

    let work_dir = cwd
        .clone()
        .or_else(|| env::current_dir().ok().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_else(|| "/".to_string());

    let mut cmd = CommandBuilder::new(&shell);
    cmd.cwd(&work_dir);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("LANG", "en_US.UTF-8");

    // Disable zsh EOL mark (%) when line doesn't end with newline
    cmd.env("PROMPT_EOL_MARK", "");

    // Shell launch mode: heavy rc config → lightweight shell (CPU fix); light
    // or missing config → full interactive shell (no functional regression).
    let shell_mode = detect_shell_mode(&shell);

    match shell_mode {
        ShellMode::Interactive => {
            // Interactive shell for proper prompt and aliases
            if !cfg!(target_os = "windows") {
                if shell.ends_with("zsh") || shell.ends_with("bash") {
                    cmd.arg("-i");
                }
            }
        }
        ShellMode::Lightweight => {
            // Skip rc loading entirely — this is what stops idle shells from
            // burning CPU on prompt polling (git status, precmd hooks, plugins).
            if shell.ends_with("zsh") {
                cmd.arg("-f");
            } else if shell.ends_with("bash") {
                cmd.arg("--noprofile");
                cmd.arg("--norc");
            }
            // The rc file normally restores the user's PATH (homebrew/nvm/etc).
            // Recover it from the interactive shell so tools still resolve.
            if let Some(path) = detect_user_path(&shell) {
                cmd.env("PATH", path);
            }
        }
    }

    let _child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    let master = pair.master;

    // Clone reader side for dedicated read thread
    let mut reader = master.try_clone_reader().map_err(|e| e.to_string())?;

    let app_clone = app.clone();
    let mut ts = state.lock().unwrap();
    let id = ts.next_id;
    ts.next_id += 1;

    let event_name = format!("terminal-data-{}", id);
    // Buffer shared with the read thread; drained by take_terminal_pending when
    // the frontend mounts the xterm. Kept in the slot so the command can reach it.
    let pending = Arc::new(Mutex::new(Vec::new()));
    let pending_thread = pending.clone();
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = app_clone.emit(&event_name, b"\r\n\x1b[31m[Process exited]\x1b[0m\r\n".to_vec());
                    break;
                }
                Ok(n) => {
                    // Buffer pre-mount output (fixes lost history on fast session
                    // switches: the frontend drains this once its listener is up).
                    {
                        let mut p = pending_thread.lock().unwrap();
                        p.extend_from_slice(&buf[..n]);
                        if p.len() > MAX_PENDING_BYTES {
                            let excess = p.len() - MAX_PENDING_BYTES;
                            p.drain(..excess);
                        }
                    }
                    let _ = app_clone.emit(&event_name, buf[..n].to_vec());
                }
                Err(_) => {
                    let _ = app_clone.emit(&event_name, b"\r\n\x1b[31m[Terminal closed]\x1b[0m\r\n".to_vec());
                    break;
                }
            }
        }
    });

    ts.terminals.insert(id, TerminalSlot {
        writer: master,
        cwd: work_dir,
        _handle: handle,
        pending,
    });

    Ok(id)
}

/// Drain the pre-mount PTY output buffer for a terminal. The frontend calls
/// this from mountTerminal AFTER attaching its data listener (so nothing falls
/// between the drain and the listener), then writes the bytes as part of the
/// history restore. Without this, a terminal whose shell produced output before
/// the xterm existed shows up empty after a rapid session switch.
#[tauri::command]
pub fn take_terminal_pending(
    state: tauri::State<'_, Mutex<TerminalState>>,
    terminal_id: u32,
) -> Result<Vec<u8>, String> {
    let ts = state.lock().unwrap();
    if let Some(slot) = ts.terminals.get(&terminal_id) {
        let mut p = slot.pending.lock().unwrap();
        Ok(p.drain(..).collect())
    } else {
        Err(format!("Terminal {} not found", terminal_id))
    }
}

#[tauri::command]
pub fn write_terminal(
    state: tauri::State<'_, Mutex<TerminalState>>,
    terminal_id: u32,
    data: Vec<u8>,
) -> Result<(), String> {
    let mut ts = state.lock().unwrap();
    if let Some(slot) = ts.terminals.get_mut(&terminal_id) {
        slot.writer.write_all(&data).map_err(|e| e.to_string())
    } else {
        Err(format!("Terminal {} not found", terminal_id))
    }
}

#[tauri::command]
pub fn kill_terminal(
    state: tauri::State<'_, Mutex<TerminalState>>,
    terminal_id: u32,
) -> Result<(), String> {
    let mut ts = state.lock().unwrap();
    // Dropping writer closes stdin to shell, shell should exit
    ts.terminals.remove(&terminal_id);
    Ok(())
}

#[tauri::command]
pub fn resize_terminal(
    state: tauri::State<'_, Mutex<TerminalState>>,
    terminal_id: u32,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let ts = state.lock().unwrap();
    if let Some(slot) = ts.terminals.get(&terminal_id) {
        slot.writer
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| e.to_string())
    } else {
        Err(format!("Terminal {} not found", terminal_id))
    }
}

/// Report the shell mode the terminal panel is running under, so the frontend
/// can explain to the user why their shell looks "bare" (lightweight mode).
#[tauri::command]
pub fn get_terminal_shell_mode() -> String {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    match detect_shell_mode(&shell) {
        ShellMode::Lightweight => "lightweight".to_string(),
        ShellMode::Interactive => "interactive".to_string(),
    }
}

/// Terminate every backend terminal shell. Called when the app window closes —
/// without this, shells spawned for the terminal panel stay alive as orphaned
/// processes (each interactive zsh may keep polling prompt state, burning CPU),
/// and they accumulate across app restarts because the frontend module state
/// (tabs/history) is wiped on launch, so the shells can never be reused.
pub fn kill_all_terminals(app: &tauri::AppHandle) {
    use tauri::Manager;
    let ts = app.state::<Mutex<TerminalState>>();
    // Dropping every master closes the PTY → SIGHUP to each shell. The read
    // threads then observe read errors/EOF and exit.
    ts.lock().unwrap().terminals.clear();
}

#[tauri::command]
pub fn get_terminal_cwd(
    state: tauri::State<'_, Mutex<TerminalState>>,
    terminal_id: u32,
) -> Result<String, String> {
    let ts = state.lock().unwrap();
    if let Some(slot) = ts.terminals.get(&terminal_id) {
        Ok(slot.cwd.clone())
    } else {
        Err(format!("Terminal {} not found", terminal_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_short_or_empty_is_light() {
        assert!(!rc_content_is_heavy(""));
        assert!(!rc_content_is_heavy("# a comment\nalias ll='ls -l'\nexport EDITOR=vim\n"));
    }

    #[test]
    fn rc_with_expensive_markers_is_heavy() {
        for marker in ["oh-my-zsh", "powerlevel10k", "p10k", "starship",
                       "zsh-syntax-highlighting", "zsh-autosuggestions",
                       "antigen", "zplug", "zinit", "sheldon", "fzf",
                       "nvm.sh", "conda", "pyenv", "rvm", "fnm", "mise"] {
            let content = format!("# config\nsource ~/.{marker}/something\n");
            assert!(rc_content_is_heavy(&content), "marker {marker} not detected");
        }
    }

    #[test]
    fn rc_with_nvm_or_conda_loader_is_heavy() {
        // Real-world minimal rc that still costs CPU on every shell start.
        let nvm = "export NVM_DIR=\"$HOME/.nvm\"\n[ -s \"$NVM_DIR/nvm.sh\" ] && \\. \"$NVM_DIR/nvm.sh\"\n";
        assert!(rc_content_is_heavy(nvm));
        let conda = "__conda_setup=\"$(conda shell.zsh hook)\"\neval \"$__conda_setup\"\n";
        assert!(rc_content_is_heavy(conda));
    }

    #[test]
    fn rc_over_60_lines_is_heavy() {
        let content = (0..61).map(|i| format!("export FOO{i}=1")).collect::<Vec<_>>().join("\n");
        assert!(rc_content_is_heavy(&content));
    }

    #[test]
    fn rc_under_60_plain_lines_is_light() {
        let content = (0..40).map(|i| format!("export FOO{i}=1")).collect::<Vec<_>>().join("\n");
        assert!(!rc_content_is_heavy(&content));
    }

    #[test]
    fn non_shell_returns_interactive() {
        assert_eq!(detect_shell_mode("cmd.exe"), ShellMode::Interactive);
        assert_eq!(detect_shell_mode("/bin/fish"), ShellMode::Interactive);
    }
}

    /// Manual integration check: verify the real $SHELL gets detected as
    /// Lightweight when the rc contains nvm/conda, and that the probed PATH is
    /// non-empty and contains expected entries. Run with:
    ///   cargo test --lib manual_ -- --ignored --nocapture
    #[test]
    #[ignore]
    fn manual_real_shell_detection() {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mode = detect_shell_mode(&shell);
        let path = detect_user_path(&shell);
        println!("shell={shell} mode={mode:?} path={path:?}");
        assert!(path.as_deref().is_some_and(|p| !p.is_empty()), "PATH probe failed");
        assert!(path.as_deref().is_some_and(|p| p.contains('/')), "PATH has no entries");
    }
