# Plan: 修复终端历史丢失 + 降低终端 CPU 占用

Date: 2026-08-20

## 背景

用户报告两个问题（文件树视图下的终端面板）：

1. **历史丢失**：会话 A 打开终端 → 切到会话 B → 切回 A，之前的历史消息全部丢失。
2. **CPU 高**：打开终端后系统相关进程 CPU 明显升高。

## 根因（已诊断）

### Q1 历史丢失

`TerminalPanel.vue` 的终端创建链路是 fire-and-forget 的：

- `addTab()` → `createTab()`（后端 spawn PTY，返回 id）→ push tab（此时 `tab.term === null`）→ `mountTerminal(tab)` 异步建 xterm。
- `mountTerminal` 内 `await listen(...)` + `term.open()` + 写 history，整个过程 ~50-300ms。
- 会话切换 `watch(() => props.cwd)` 中调用 `saveDirectoryState(oldCwd)`：若切换发生在 `mountTerminal` 完成前，`tab.term === null`，`captureBufferText` 走不到，`bufferText` 落空 → 切回时恢复为空。

### Q2 CPU 高

`term_cmd.rs` spawn 的是 **interactive shell**（`zsh -i` / `bash -i`），加载完整 `~/.zshrc`（oh-my-zsh / p10k / starship / syntax-highlighting 等），每个 idle shell 也会因 prompt 异步刷新（git status 轮询、precmd hooks）持续吃 CPU。每开过一个会话的目录就留一个 interactive zsh，N 个会话 = N 倍 CPU。

## 修复方案（用户已批准）

- **Q1**：后端把 PTY 输出在 mount 前缓冲到内存，前端 mount 时取回合并写入 → 彻底消除竞态。
- **Q2**：探测用户 rc 配置，配置重 → spawn 轻量 shell（zsh -f / bash --noprofile --norc）并注入从交互 shell 探测的 PATH；配置轻 → 维持 interactive。前端弹一次性 toast 提示。

---

## 任务 1：后端 PTY 输出缓冲（Q1）

### 文件：`src-tauri/src/commands/term_cmd.rs`

1. `TerminalSlot` 增加字段 `pending: Arc<Mutex<Vec<u8>>>`。
2. `spawn_terminal`：
   - 创建 `pending = Arc::new(Mutex::new(Vec::new()))`。
   - 读线程中读到数据后，先写入 `pending`（cap 1 MiB，超限丢最旧）再 emit 前端。
   - slot 中保存 `pending` 的 Arc。
3. 新命令 `take_terminal_pending(terminal_id: u32) -> Result<Vec<u8>, String>`：drain 并返回该 slot 的 pending 缓冲。

### 文件：`src-tauri/src/lib.rs`

4. 注册 `commands::term_cmd::take_terminal_pending`。

### 文件：`src/components/TerminalPanel.vue`

5. `mountTerminal`：在 `await listen(...)` 之后（保证 listen 之后到达的数据走事件通道、无丢失窗口）、写 history 之前，调用 `invoke("take_terminal_pending", { terminalId: tab.id })` 取回 Uint8Array。取回后重新检查 generation guard（take 是 await，期间可能被 supersede）。
6. 写入顺序：`pendingBuffer`（保存的历史）→ `pendingData`（切换期间的 PTY 输出）→ `pendingOutput`（挂载后的实时输出）→ `historyDone` 置位。

## 任务 2：轻量 shell 模式（Q2）

### 文件：`src-tauri/src/commands/term_cmd.rs`

7. 新增 `pub enum ShellMode { Interactive, Lightweight }`。
8. 纯函数 `rc_content_is_heavy(content: &str) -> bool`（可单测）：行数 > 60 或含重型关键词（oh-my-zsh、powerlevel10k、p10k、starship、zsh-syntax-highlighting、zsh-autosuggestions、antigen、zplug、zinit、sheldon、fzf）。
9. `pub fn detect_shell_mode(shell: &str) -> ShellMode`：按 $SHELL 读取 `~/.zshrc` / `~/.bashrc`，重 → Lightweight，否则 Interactive。
10. `fn detect_user_path(shell: &str) -> Option<String>`（`OnceLock` 缓存）：执行一次 `$SHELL -ic 'printf %s "$PATH"'`，2s 超时 kill，取 stdout。解决 GUI 进程 PATH 不含 homebrew/nvm 的问题。
11. `spawn_terminal` 分支：
    - `Interactive`：维持 `-i`（现状）。
    - `Lightweight`：zsh → `-f`；bash → `--noprofile --norc`；并 `cmd.env("PATH", detect_user_path(shell)?)`（探测失败则保留默认）。
12. 新命令 `get_terminal_shell_mode() -> String`（"lightweight" | "interactive"），注册到 lib.rs。

### 文件：`src/components/TerminalPanel.vue`

13. 模块级 once flag + `maybeHintLightweightShell()`：首次 init 时 fire-and-forget 调用 `get_terminal_shell_mode`，若为 lightweight 用 `useToast().showWarning(...)` 提示一次（"你的 shell 配置较重，终端已用轻量模式以降低 CPU 占用"）。

## 测试

### Rust 单测（`term_cmd.rs` 内 `#[cfg(test)] mod tests`）

- `rc_content_is_heavy`：
  - 空/短内容 → false
  - 含 "oh-my-zsh" / "powerlevel10k" / "starship" → true
  - 61 行普通内容 → true；40 行 → false

### 手动验证

1. `cargo test -p runjam`（crate `runjam`，lib 名 `runjam_lib`）。
2. `cargo check`。
3. `npm run build`（vue-tsc 类型检查）。
4. 运行时验证（需要用户操作）：
   - 开终端 → 立即切会话 → 切回 → 历史保留（快速切换场景，之前必现）
   - 开终端等 1s → 切会话 → 切回 → 历史保留
   - 打开多个会话终端后，Activity Monitor 观察 zsh 进程 CPU 显著下降

## 边界情况

- `detect_user_path` 超时/失败 → 回退默认 PATH（GUI 默认 PATH），shell 仍能启动。
- pending 缓冲 cap 1 MiB，超限丢弃最旧（防长期不 mount 的终端无限增长）。
- `take_terminal_pending` 在 mount 前被 kill 的终端 → 返回 Err，前端 catch 忽略。
- Windows：`cmd.exe` 分支完全不受影响（无 -f / rc 概念，PATH 用系统环境）。
- 注意：工作区已有其他未提交修改（搜索相关），本次只触碰终端相关文件，不打包其他改动。
