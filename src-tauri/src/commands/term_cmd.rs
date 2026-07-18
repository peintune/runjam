use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::thread;
use std::env;
use tauri::{AppHandle, Emitter};

struct TerminalSlot {
    writer: Box<dyn MasterPty + Send>,
    cwd: String,
    _handle: thread::JoinHandle<()>,
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

    // Interactive shell for proper prompt and aliases
    if !cfg!(target_os = "windows") {
        if shell.ends_with("zsh") || shell.ends_with("bash") {
            cmd.arg("-i");
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
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = app_clone.emit(&event_name, b"\r\n\x1b[31m[Process exited]\x1b[0m\r\n".to_vec());
                    break;
                }
                Ok(n) => {
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
    });

    Ok(id)
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
    _state: tauri::State<'_, Mutex<TerminalState>>,
    _terminal_id: u32,
    _rows: u16,
    _cols: u16,
) -> Result<(), String> {
    Ok(())
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
