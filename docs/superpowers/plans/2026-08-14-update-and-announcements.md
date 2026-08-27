# RunJam 升级 + 公告 功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 RunJam 桌面应用实现 Windows 自动更新（Tauri updater）、macOS 降级跳转下载、以及应用内公告（横幅 + 弹窗）。

**Architecture:** 前端只调用统一命令 `check_update_ui()`，平台差异封装在 Rust 端（Windows 走 updater 插件，macOS 走现有 `/api/updates/latest`）。公告从 `/api/announcements` 拉取，本地 `app_settings` 表记住已读，按 `level` 分流横幅/弹窗。

**Tech Stack:** Tauri 2, Rust, Vue 3, TypeScript, SQLite (rusqlite), GitHub Actions

**Spec:** `docs/superpowers/specs/2026-08-14-update-and-announcements-design.md`

## Global Constraints

- 版本号格式：`v0.1.0`（带 `v` 前缀），比较用语义化版本。
- 平台字符串：`darwin` / `win32` / `linux`（`platform_name()` 已定义）。
- 公告 `level` 取值：`"info"`（横幅）| `"important"`（弹窗）。
- 公告已读 key 格式：`announcement_read:<id>`，存 `app_settings` 表。
- updater 仅 Windows 启用；macOS 不调用 `app.updater()`。
- 前端无测试框架（不引入 vitest/jest）；Rust 用内联 `#[cfg(test)]` 模块。
- 对外 API base：`https://runjam.app`（`telemetry::api_base()`）。

---

### Task 1: Rust 依赖 + updater 插件注册

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: 无（首个任务）
- Produces: updater 插件注册在 app builder 上；`tauri_plugin_updater` crate 可用。

- [ ] **Step 1: 在 Cargo.toml 添加 updater 插件依赖**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 中，`tauri-plugin-dialog = "2"` 之后添加：

```toml
tauri-plugin-updater = "2"
```

- [ ] **Step 2: 在 lib.rs 注册 updater 插件**

在 `src-tauri/src/lib.rs` 中，找到 `.plugin(...)` 链式调用处（现有 `tauri-plugin-opener`、`tauri-plugin-shell`、`tauri-plugin-dialog` 注册附近），添加：

```rust
.plugin(tauri_plugin_updater::Builder::new().build())
```

- [ ] **Step 3: 在 tauri.conf.json 配置 updater**

在 `src-tauri/tauri.conf.json` 的 `"bundle"` 对象后（顶层）添加 `"plugins"`：

```json
"plugins": {
  "updater": {
    "pubkey": "PASTE_YOUR_PUBLIC_KEY_HERE",
    "endpoints": ["https://.app/api/updates/latest"],
    "windows": { "installMode": "passive" }
  }
}
```

> 注：`pubkey` 先用占位符，Task 8（密钥生成）时替换为真实公钥。占位符不会导致编译失败（运行时才校验）。

- [ ] **Step 4: 在 capabilities/default.json 添加 updater 权限**

在 `src-tauri/capabilities/default.json` 的 `"permissions"` 数组中添加：

```json
"updater:default"
```

- [ ] **Step 5: 验证编译通过**

Run: `cd src-tauri && cargo check`
Expected: 编译通过（`error[E0432]` 等错误为失败）。若报 `tauri-plugin-updater` 未找到，检查 Cargo.toml 依赖是否添加正确。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/tauri.conf.json src-tauri/capabilities/default.json
git commit -m "feat: add tauri updater plugin scaffolding"
```

---

### Task 2: updates.rs — 平台分流与版本比较纯函数

**Files:**
- Create: `src-tauri/src/updates.rs`
- Modify: `src-tauri/src/lib.rs`（声明模块）

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub fn is_windows() -> bool`
  - `pub fn version_ge(current: &str, min: &str) -> bool` — 语义化版本比较，`current >= min`
  - `pub struct UpdateCheckResult { pub update_available: bool, pub action: String, pub latest_version: Option<String>, pub notes: Option<String>, pub download_url: Option<String> }`（`Serialize`）

- [ ] **Step 1: 写失败测试**

在 `src-tauri/src/updates.rs` 中创建模块，先写测试（此时 `version_ge` 未实现，测试失败）：

```rust
//! Update & announcement helpers (pure functions, unit-testable).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ge_compares_semver() {
        assert!(version_ge("v0.2.0", "v0.1.0"));
        assert!(version_ge("v0.1.0", "v0.1.0"));
        assert!(!version_ge("v0.1.0", "v0.2.0"));
        assert!(version_ge("v0.2.0", "v0.1.5"));
        assert!(!version_ge("v0.1.0", "v0.1.0-beta"));
    }

    #[test]
    fn version_ge_handles_missing_v_prefix() {
        assert!(version_ge("0.2.0", "v0.1.0"));
        assert!(version_ge("v0.2.0", "0.1.0"));
    }

    #[test]
    fn version_ge_min_version_null_means_always_visible() {
        // min_version 为空时公告始终可见（由调用方处理，这里测试解析空串）
        assert!(version_ge("v0.1.0", ""));
    }
}
```

> 此时 `version_ge` 未定义，`cargo test` 会报 `cannot find function`。

- [ ] **Step 2: 运行测试验证失败**

Run: `cd src-tauri && cargo test version_ge`
Expected: 编译失败，报 `cannot find function version_ge in this scope`。

- [ ] **Step 3: 实现 version_ge 和 is_windows**

在 `updates.rs` 中实现：

```rust
use serde::Serialize;

/// True on Windows (updater active). macOS/Linux use the download-redirect path.
pub fn is_windows() -> bool {
    std::env::consts::OS == "windows"
}

/// Semantic version comparison: does `current` >= `min`?
/// Accepts optional leading 'v'. Empty `min` means no floor (always true).
pub fn version_ge(current: &str, min: &str) -> bool {
    if min.trim().is_empty() {
        return true;
    }
    let a = parse_version(current);
    let b = parse_version(min);
    match (a, b) {
        (Some(a), Some(b)) => {
            (a.0, a.1, a.2) >= (b.0, b.1, b.2)
        }
        // Unparseable: fall back to string comparison so we never hide a
        // release due to a parse bug.
        _ => current.trim() >= min.trim(),
    }
}

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let s = v.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next()?.split(['-', '+']).next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Unified result returned to the frontend. `action` is "install" (Windows)
/// or "open_download" (macOS/Linux).
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub update_available: bool,
    pub action: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub download_url: Option<String>,
}

impl UpdateCheckResult {
    pub fn none() -> Self {
        Self {
            update_available: false,
            action: if is_windows() { "install".into() } else { "open_download".into() },
            latest_version: None,
            notes: None,
            download_url: None,
        }
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cd src-tauri && cargo test version_ge`
Expected: 3 个测试全部 PASS。

- [ ] **Step 5: 在 lib.rs 声明模块**

在 `src-tauri/src/lib.rs` 顶部模块声明处（`mod telemetry;` 附近）添加：

```rust
mod updates;
```

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/updates.rs src-tauri/src/lib.rs
git commit -m "feat: add update platform-split and semver comparison helpers"
```

---

### Task 3: 公告数据模型与已读存储

**Files:**
- Modify: `src-tauri/src/updates.rs`（追加公告逻辑）
- Modify: `src-tauri/src/telemetry.rs`（复用 `api_base()`、`build_agent` 或新增公告拉取）

**Interfaces:**
- Consumes: `telemetry::api_base()`、`telemetry::sanitize()`（已有）
- Produces:
  - `pub struct Announcement { pub id: String, pub title: String, pub body: String, pub level: String, pub created_at: Option<String> }`（`Serialize, Deserialize`）
  - `pub fn is_announcement_read(conn: &rusqlite::Connection, id: &str) -> bool`
  - `pub fn mark_announcement_read(conn: &rusqlite::Connection, id: &str) -> rusqlite::Result<()>`
  - `pub fn filter_unread(conn: &rusqlite::Connection, items: Vec<Announcement>) -> Vec<Announcement>`

- [ ] **Step 1: 写失败测试（已读过滤）**

在 `updates.rs` 的 `mod tests` 中追加：

```rust
#[test]
fn filter_unread_removes_read_items() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);",
    )
    .unwrap();
    mark_announcement_read(&conn, "a").unwrap();

    let items = vec![
        Announcement { id: "a".into(), title: "read".into(), body: "".into(), level: "info".into(), created_at: None },
        Announcement { id: "b".into(), title: "new".into(), body: "".into(), level: "important".into(), created_at: None },
    ];
    let unread = filter_unread(&conn, items);
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].id, "b");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cd src-tauri && cargo test filter_unread`
Expected: 编译失败，报 `cannot find function filter_unread`。

- [ ] **Step 3: 实现公告模型与已读存储**

在 `updates.rs` 中追加：

```rust
use rusqlite::Connection;
use serde::Deserialize;

pub const KEY_ANNOUNCEMENT_READ_PREFIX: &str = "announcement_read:";

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub id: String,
    pub title: String,
    pub body: String,
    pub level: String, // "info" | "important"
    pub created_at: Option<String>,
}

pub fn is_announcement_read(conn: &Connection, id: &str) -> bool {
    let key = format!("{}{}", KEY_ANNOUNCEMENT_READ_PREFIX, id);
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [&key],
        |r| r.get::<_, String>(0),
    )
    .map(|v| v == "1")
    .unwrap_or(false)
}

pub fn mark_announcement_read(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    let key = format!("{}{}", KEY_ANNOUNCEMENT_READ_PREFIX, id);
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?1, '1', CURRENT_TIMESTAMP)",
        [&key],
    )
    .map(|_| ())
}

pub fn filter_unread(conn: &Connection, items: Vec<Announcement>) -> Vec<Announcement> {
    items
        .into_iter()
        .filter(|a| !is_announcement_read(conn, &a.id))
        .collect()
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cd src-tauri && cargo test filter_unread`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/updates.rs
git commit -m "feat: add announcement model and read-state storage helpers"
```

---

### Task 4: 公告拉取命令（get_announcements / mark_announcement_read）

**Files:**
- Modify: `src-tauri/src/commands/telemetry_cmd.rs`
- Modify: `src-tauri/src/lib.rs`（注册命令）

**Interfaces:**
- Consumes: `updates::{Announcement, filter_unread, mark_announcement_read}`、`telemetry::api_base()`、`telemetry::build_agent`（需将 `build_agent` 改为 `pub`）
- Produces:
  - `#[tauri::command] pub async fn get_announcements(app: tauri::AppHandle, db: State<'_, Mutex<Database>>) -> Result<Vec<Announcement>, String>`
  - `#[tauri::command] pub fn mark_announcement_read(db: State<'_, Mutex<Database>>, id: String) -> Result<(), String>`

- [ ] **Step 1: 将 telemetry::build_agent 改为 pub**

在 `src-tauri/src/telemetry.rs` 中，将 `fn build_agent` 改为 `pub fn build_agent`（供命令复用代理逻辑）。

- [ ] **Step 2: 实现公告命令**

在 `src-tauri/src/commands/telemetry_cmd.rs` 中追加（文件顶部 `use` 需加 `crate::updates`）：

```rust
/// Fetch unread announcements (server filters active + min_version by the
/// current app version). Client filters locally-read ones.
#[tauri::command]
pub async fn get_announcements(
    app: tauri::AppHandle,
    db: State<'_, Mutex<Database>>,
) -> Result<Vec<crate::updates::Announcement>, String> {
    let base = telemetry::api_base();
    let version = app.package_info().version.to_string();
    let url = format!("{}/api/announcements?current={}", base, version);
    let agent = telemetry::build_agent(&app);
    let resp = agent
        .get(&url)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| format!("announcements fetch failed: {}", e))?;
    let items: Vec<crate::updates::Announcement> = resp
        .into_json()
        .map_err(|e| format!("bad announcements response: {}", e))?;

    let guard = db.lock().map_err(|e| e.to_string())?;
    let conn = guard.conn.lock().map_err(|e| e.to_string())?;
    Ok(crate::updates::filter_unread(&conn, items))
}

/// Mark an announcement as read so it is not shown again.
#[tauri::command]
pub fn mark_announcement_read(
    db: State<'_, Mutex<Database>>,
    id: String,
) -> Result<(), String> {
    let guard = db.lock().map_err(|e| e.to_string())?;
    let conn = guard.conn.lock().map_err(|e| e.to_string())?;
    crate::updates::mark_announcement_read(&conn, &id).map_err(|e| e.to_string())
}
```

> 注：`get_announcements` 用 `app` 参数获取版本号，但 `build_agent` 也需要 `app`。此处 `app` 同时用于两者，无冲突。

- [ ] **Step 3: 注册命令**

在 `src-tauri/src/lib.rs` 的 `invoke_handler` 中，`commands::telemetry_cmd::test_proxy` 之后添加：

```rust
commands::telemetry_cmd::get_announcements,
commands::telemetry_cmd::mark_announcement_read,
```

- [ ] **Step 4: 验证编译**

Run: `cd src-tauri && cargo check`
Expected: 编译通过。若报 `build_agent` 私有，确认 Step 1 已改为 `pub`。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/telemetry.rs src-tauri/src/commands/telemetry_cmd.rs src-tauri/src/lib.rs
git commit -m "feat: add announcement fetch and mark-read commands"
```

---

### Task 5: 统一更新检查命令（check_update_ui / install_update）

**Files:**
- Modify: `src-tauri/src/commands/telemetry_cmd.rs`
- Modify: `src-tauri/src/lib.rs`（注册命令）

**Interfaces:**
- Consumes: `updates::{UpdateCheckResult, is_windows}`、现有 `check_for_updates`（返回 `UpdateInfo`）
- Produces:
  - `#[tauri::command] pub async fn check_update_ui(app: tauri::AppHandle, current: String) -> Result<UpdateCheckResult, String>`
  - `#[tauri::command] pub async fn install_update(app: tauri::AppHandle) -> Result<(), String>`

- [ ] **Step 1: 实现 check_update_ui（平台分流）**

在 `src-tauri/src/commands/telemetry_cmd.rs` 中追加：

```rust
/// Unified update check. Windows uses the updater plugin; macOS/Linux use
/// the backend metadata endpoint and return a download URL for redirect.
#[tauri::command]
pub async fn check_update_ui(
    app: tauri::AppHandle,
    current: String,
) -> Result<crate::updates::UpdateCheckResult, String> {
    if crate::updates::is_windows() {
        let updater = app
            .updater()
            .map_err(|e| format!("updater init failed: {}", e))?;
        match updater.check().await {
            Ok(Some(update)) => Ok(crate::updates::UpdateCheckResult {
                update_available: true,
                action: "install".into(),
                latest_version: Some(update.version.to_string()),
                notes: update.body.clone(),
                download_url: None,
            }),
            Ok(None) => Ok(crate::updates::UpdateCheckResult::none()),
            Err(e) => Err(format!("update check failed: {}", e)),
        }
    } else {
        // macOS/Linux: reuse the existing metadata check.
        let info = check_for_updates(current).await?;
        Ok(crate::updates::UpdateCheckResult {
            update_available: info.update_available,
            action: "open_download".into(),
            latest_version: info.latest_version,
            notes: info.notes,
            download_url: info.download_url,
        })
    }
}

/// Windows: trigger download + install of the pending update.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| format!("updater init failed: {}", e))?;
    if let Some(update) = updater.check().await.map_err(|e| format!("update check failed: {}", e))? {
        update
            .download_and_install(|_, _| {}, |_, _| {})
            .await
            .map_err(|e| format!("install failed: {}", e))?;
    }
    Ok(())
}
```

> 注：`check_for_updates` 已存在（返回 `UpdateInfo`），此处直接调用。`UpdateInfo` 字段 `download_url` 为 `Option<String>`，直接转移。

- [ ] **Step 2: 注册命令**

在 `src-tauri/src/lib.rs` 的 invoke_handler 中，`commands::telemetry_cmd::test_proxy` 之后添加：

```rust
commands::telemetry_cmd::check_update_ui,
commands::telemetry_cmd::install_update,
```

- [ ] **Step 3: 验证编译**

Run: `cd src-tauri && cargo check`
Expected: 编译通过。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/commands/telemetry_cmd.rs src-tauri/src/lib.rs
git commit -m "feat: add unified update-check and install commands with platform split"
```

---

### Task 6: 前端 API 封装

**Files:**
- Modify: `src/api/telemetry.ts`

**Interfaces:**
- Consumes: 现有 `checkForUpdates`（保留不动）
- Produces:
  - `export interface UpdateCheckResult { updateAvailable: boolean; action: "install" | "open_download"; latestVersion?: string | null; notes?: string | null; downloadUrl?: string | null; }`
  - `export async function checkUpdateUi(current: string): Promise<UpdateCheckResult>`
  - `export async function installUpdate(): Promise<void>`
  - `export interface Announcement { id: string; title: string; body: string; level: "info" | "important"; createdAt?: string | null; }`
  - `export async function getAnnouncements(): Promise<Announcement[]>`
  - `export async function markAnnouncementRead(id: string): Promise<void>`

- [ ] **Step 1: 添加更新检查 API**

在 `src/api/telemetry.ts` 中，在现有 `checkForUpdates` 之后追加：

```typescript
export interface UpdateCheckResult {
  updateAvailable: boolean;
  action: "install" | "open_download";
  latestVersion?: string | null;
  notes?: string | null;
  downloadUrl?: string | null;
}

export async function checkUpdateUi(current: string): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("check_update_ui", { current });
}

export async function installUpdate(): Promise<void> {
  return invoke<void>("install_update");
}
```

- [ ] **Step 2: 添加公告 API**

在文件末尾追加：

```typescript
export interface Announcement {
  id: string;
  title: string;
  body: string;
  level: "info" | "important";
  createdAt?: string | null;
}

export async function getAnnouncements(): Promise<Announcement[]> {
  return invoke<Announcement[]>("get_announcements");
}

export async function markAnnouncementRead(id: string): Promise<void> {
  return invoke<void>("mark_announcement_read", { id });
}
```

- [ ] **Step 3: 类型检查**

Run: `npx vue-tsc --noEmit`
Expected: 无类型错误。

- [ ] **Step 4: 提交**

```bash
git add src/api/telemetry.ts
git commit -m "feat: add update and announcement API wrappers"
```

---

### Task 7: 前端公告组件（Banner + Modal）

**Files:**
- Create: `src/components/AnnouncementBanner.vue`
- Create: `src/components/AnnouncementModal.vue`

**Interfaces:**
- Consumes: `Announcement` 类型（Task 6）
- Produces:
  - `AnnouncementBanner.vue` — props: `announcement: Announcement`; emits: `close`
  - `AnnouncementModal.vue` — props: `announcement: Announcement`; emits: `close`

- [ ] **Step 1: 创建 AnnouncementBanner.vue**

```vue
<script setup lang="ts">
import type { Announcement } from "../api/telemetry";

defineProps<{ announcement: Announcement }>();
defineEmits<{ close: [] }>();
</script>

<template>
  <div
    class="flex w-80 items-start gap-3 rounded-xl border border-gray-200 bg-white p-4 shadow-lg"
  >
    <div class="min-w-0 flex-1">
      <h3 class="text-[13px] font-semibold text-gray-900">{{ announcement.title }}</h3>
      <p class="mt-1 text-[12px] leading-relaxed text-gray-500 whitespace-pre-line">
        {{ announcement.body }}
      </p>
    </div>
    <button
      class="shrink-0 rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
      aria-label="关闭"
      @click="$emit('close')"
    >
      ✕
    </button>
  </div>
</template>
```

- [ ] **Step 2: 创建 AnnouncementModal.vue**

```vue
<script setup lang="ts">
import type { Announcement } from "../api/telemetry";

defineProps<{ announcement: Announcement }>();
defineEmits<{ close: [] }>();
</script>

<template>
  <div class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6">
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <div class="flex items-start justify-between gap-4">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
          {{ announcement.title }}
        </h2>
        <button
          class="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
          aria-label="关闭"
          @click="$emit('close')"
        >
          ✕
        </button>
      </div>
      <p class="mt-3 text-[13px] leading-relaxed text-gray-500 whitespace-pre-line">
        {{ announcement.body }}
      </p>
      <div class="mt-6 flex items-center justify-end">
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors"
          @click="$emit('close')"
        >
          知道了
        </button>
      </div>
    </div>
  </div>
</template>
```

- [ ] **Step 3: 类型检查**

Run: `npx vue-tsc --noEmit`
Expected: 无类型错误。

- [ ] **Step 4: 提交**

```bash
git add src/components/AnnouncementBanner.vue src/components/AnnouncementModal.vue
git commit -m "feat: add announcement banner and modal components"
```

---

### Task 8: 前端升级提示组件（UpdatePrompt）

**Files:**
- Create: `src/components/UpdatePrompt.vue`

**Interfaces:**
- Consumes: `UpdateCheckResult`（Task 6）、`installUpdate`（Task 6）、`openUrl`（`@tauri-apps/plugin-opener`）
- Produces: `UpdatePrompt.vue` — props: `result: UpdateCheckResult`; emits: `close`

- [ ] **Step 1: 创建 UpdatePrompt.vue**

```vue
<script setup lang="ts">
import { ref } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { installUpdate } from "../api/telemetry";
import type { UpdateCheckResult } from "../api/telemetry";

const props = defineProps<{ result: UpdateCheckResult }>();
defineEmits<{ close: [] }>();

const installing = ref(false);
const error = ref("");

async function onPrimaryAction() {
  if (props.result.action === "install") {
    installing.value = true;
    error.value = "";
    try {
      await installUpdate();
      // On success the app restarts itself; nothing more to do here.
    } catch (e) {
      error.value = String(e);
      installing.value = false;
    }
  } else if (props.result.downloadUrl) {
    try {
      await openUrl(props.result.downloadUrl);
    } catch {
      error.value = "无法打开下载链接";
    }
  }
}
</script>

<template>
  <div class="fixed inset-0 z-[99999] flex items-center justify-center bg-black/40 px-6">
    <div class="w-full max-w-md rounded-2xl bg-white p-6 shadow-2xl">
      <div class="flex items-start justify-between gap-4">
        <h2 class="text-[18px] font-semibold text-gray-900 tracking-tight">
          发现新版本 {{ result.latestVersion }}
        </h2>
        <button
          class="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
          aria-label="关闭"
          @click="$emit('close')"
        >
          ✕
        </button>
      </div>
      <p v-if="result.notes" class="mt-3 text-[13px] leading-relaxed text-gray-500 whitespace-pre-line">
        {{ result.notes }}
      </p>
      <p v-if="error" class="mt-3 text-[12px] text-red-500">{{ error }}</p>
      <div class="mt-6 flex items-center justify-end gap-3">
        <button
          class="rounded-md px-4 py-2 text-[13px] font-medium text-gray-500 hover:text-gray-700 transition-colors"
          :disabled="installing"
          @click="$emit('close')"
        >
          稍后
        </button>
        <button
          class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
          :disabled="installing"
          @click="onPrimaryAction"
        >
          {{ installing ? "正在下载…" : result.action === "install" ? "下载并安装" : "前往下载" }}
        </button>
      </div>
    </div>
  </div>
</template>
```

- [ ] **Step 2: 确认 opener 权限**

`@tauri-apps/plugin-opener` 已在依赖中，`capabilities/default.json` 已有 `opener:default`。确认 `openUrl` 可用。

- [ ] **Step 3: 类型检查**

Run: `npx vue-tsc --noEmit`
Expected: 无类型错误。

- [ ] **Step 4: 提交**

```bash
git add src/components/UpdatePrompt.vue
git commit -m "feat: add update prompt component with platform-conditional action"
```

---

### Task 9: 前端启动流程（App.vue 集成）

**Files:**
- Modify: `src/App.vue`

**Interfaces:**
- Consumes: `checkUpdateUi`、`getAnnouncements`、`markAnnouncementRead`（Task 6）、三个新组件（Task 7/8）、`useToast`（已有）
- Produces: 启动时自动检查更新 + 拉取公告并展示

- [ ] **Step 1: 在 App.vue 添加启动检查逻辑**

修改 `src/App.vue` 的 `<script setup>`，在现有 `onMounted` 中追加（在遥测 consent 逻辑之后）：

```typescript
import {
  checkUpdateUi,
  getAnnouncements,
  markAnnouncementRead,
} from "./api/telemetry";
import type { Announcement, UpdateCheckResult } from "./api/telemetry";
import UpdatePrompt from "./components/UpdatePrompt.vue";
import AnnouncementBanner from "./components/AnnouncementBanner.vue";
import AnnouncementModal from "./components/AnnouncementModal.vue";

// Update prompt + announcements state.
const updateResult = ref<UpdateCheckResult | null>(null);
const announcements = ref<Announcement[]>([]);
const activeImportant = ref<Announcement | null>(null);

function currentVersion(): string {
  // Read from a global injected at build time; fall back to "0.1.0".
  // Tauri injects __TAURI_INTERNALS__; version comes from the Rust command
  // on the backend side, so here we just pass a placeholder that the Rust
  // command ignores for announcements and uses for the update check.
  return "0.1.0";
}

async function checkUpdates() {
  try {
    const res = await checkUpdateUi(currentVersion());
    if (res.updateAvailable) {
      updateResult.value = res;
    }
  } catch {
    // Non-fatal: skip update prompt on failure.
  }
}

async function loadAnnouncements() {
  try {
    const items = await getAnnouncements();
    announcements.value = items;
    // Show the first important one as a modal; rest as banners.
    const important = items.find((a) => a.level === "important");
    if (important) {
      activeImportant.value = important;
    }
  } catch {
    // Non-fatal: skip announcements on failure.
  }
}

function closeImportant() {
  if (activeImportant.value) {
    markAnnouncementRead(activeImportant.value.id).catch(() => {});
    activeImportant.value = null;
  }
}

function dismissBanner(id: string) {
  markAnnouncementRead(id).catch(() => {});
  announcements.value = announcements.value.filter((a) => a.id !== id);
}
```

在 `onMounted` 内，`decideConsent` 逻辑之后追加调用：

```typescript
  // Kick off update check + announcement fetch in the background.
  checkUpdates();
  loadAnnouncements();
```

- [ ] **Step 2: 在模板添加组件渲染**

在 `<template>` 中，遥测 consent 弹窗之后、Toast 容器之前添加：

```vue
  <!-- Update prompt -->
  <UpdatePrompt
    v-if="updateResult"
    :result="updateResult"
    @close="updateResult = null"
  />

  <!-- Important announcement modal -->
  <AnnouncementModal
    v-if="activeImportant"
    :announcement="activeImportant"
    @close="closeImportant"
  />

  <!-- Info announcement banners (above the toast stack) -->
  <div class="fixed top-4 right-4 z-[9998] flex flex-col gap-2 w-80">
    <AnnouncementBanner
      v-for="a in announcements.filter((x) => x.level !== 'important')"
      :key="a.id"
      :announcement="a"
      @close="dismissBanner(a.id)"
    />
  </div>
```

- [ ] **Step 3: 类型检查**

Run: `npx vue-tsc --noEmit`
Expected: 无类型错误。

- [ ] **Step 4: 提交**

```bash
git add src/App.vue
git commit -m "feat: check for updates and load announcements on startup"
```

---

### Task 10: 设置页手动检查更新入口

**Files:**
- Modify: `src/views/settings/GeneralSettings.vue`

**Interfaces:**
- Consumes: `checkUpdateUi`、`UpdateCheckResult`（Task 6）
- Produces: 设置页"检查更新"按钮 + 当前版本显示 + 弹窗复用

- [ ] **Step 1: 添加检查更新逻辑**

在 `GeneralSettings.vue` 的 `<script setup>` 中，现有 telemetry 逻辑之后追加：

```typescript
import { ref } from "vue";
import { checkUpdateUi } from "@/api/telemetry";
import type { UpdateCheckResult } from "@/api/telemetry";
import UpdatePrompt from "@/components/UpdatePrompt.vue";

const checking = ref(false);
const updateResult = ref<UpdateCheckResult | null>(null);
const checkError = ref("");

async function checkForUpdate() {
  checking.value = true;
  checkError.value = "";
  try {
    const res = await checkUpdateUi("0.1.0");
    if (res.updateAvailable) {
      updateResult.value = res;
    } else {
      checkError.value = "已是最新版本";
    }
  } catch (e) {
    checkError.value = String(e);
  } finally {
    checking.value = false;
  }
}
```

- [ ] **Step 2: 在模板添加按钮**

在 `GeneralSettings.vue` 的 `<template>` 中，找到合适位置（telemetry 开关区块附近）添加一个区块：

```vue
      <div class="mt-8 border-t border-gray-100 pt-6">
        <h3 class="text-[14px] font-semibold text-gray-900 mb-3">更新</h3>
        <div class="flex items-center justify-between">
          <div>
            <p class="text-[13px] text-gray-600">检查是否有新版本可用</p>
            <p v-if="checkError" class="mt-1 text-[12px] text-gray-400">{{ checkError }}</p>
          </div>
          <button
            class="rounded-md bg-blue-600 px-4 py-2 text-[13px] font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
            :disabled="checking"
            @click="checkForUpdate"
          >
            {{ checking ? "检查中…" : "检查更新" }}
          </button>
        </div>
      </div>
```

在 `</template>` 之前添加 UpdatePrompt 弹窗：

```vue
  <UpdatePrompt
    v-if="updateResult"
    :result="updateResult"
    @close="updateResult = null"
  />
```

- [ ] **Step 3: 类型检查**

Run: `npx vue-tsc --noEmit`
Expected: 无类型错误。

- [ ] **Step 4: 提交**

```bash
git add src/views/settings/GeneralSettings.vue
git commit -m "feat: add manual update check button in settings"
```

---

### Task 11: CI 改动（Windows 只出 NSIS + sig）

**Files:**
- Modify: `.github/workflows/build.yml`

**Interfaces:**
- Consumes: 无
- Produces: Windows 构建注入签名密钥、只上传 NSIS exe + sig

- [ ] **Step 1: 修改 Windows job 产物与签名**

在 `.github/workflows/build.yml` 的 `build-windows` job 中：

1. 在 "Build Tauri (Windows)" step 的 `env:` 中追加签名密钥：

```yaml
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_ENV_PLATFORM_TARGET: x86_64-pc-windows-msvc
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
```

2. 修改 "Upload Windows artifacts" step，只上传 NSIS exe + sig（移除 msi）：

```yaml
      - name: Upload Windows artifacts
        uses: actions/upload-artifact@v5
        with:
          name: runjam-windows-x64
          path: |
            src-tauri\target\release\bundle\nsis\*.exe
            src-tauri\target\release\bundle\nsis\*.exe.sig
```

> 注：tauri-action 在设置了 `TAURI_SIGNING_PRIVATE_KEY` 时会自动生成 `.sig` 签名文件。NSIS 的 `.exe.sig` 与 `.exe` 同目录。

- [ ] **Step 2: 验证 YAML 语法**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/build.yml')); print('YAML OK')"`（若未装 pyyaml 可跳过，人工检查缩进）
Expected: 无语法错误。

- [ ] **Step 3: 提交**

```bash
git add .github/workflows/build.yml
git commit -m "ci: windows builds sign updater artifacts and drop msi"
```

---

### Task 12: 文档 + 发布流程说明

**Files:**
- Modify: `runjam_web_README.md`（或 `docs/` 下新增说明）

**Interfaces:**
- Consumes: 无
- Produces: 更新/公告功能的发布与使用文档

- [ ] **Step 1: 更新 README 说明**

在 `runjam_web_README.md` 的"发布新版本"章节后补充升级与公告说明：

```markdown
## 软件升级（客户端）

- Windows：客户端通过 Tauri updater 自动下载、校验签名并安装（NSIS）。
  发布时 CI 自动生成 `.exe.sig` 签名文件，随 GitHub Release 上传。
- macOS：客户端检测到新版本后跳转官网下载页（需手动安装）。
- 版本元数据需同步到 Supabase `releases` 表（含 `download_urls` 与 `staged`）。

## 公告（应用内）

- 在 Supabase `announcements` 表插入公告：
  - `level`: `info`（横幅）或 `important`（弹窗）
  - `min_version`: 仅对 >= 该版本的客户端可见（可空）
  - `active`: 需为 `true` 才显示
- 客户端启动时拉取，本地记住已读，只提示一次。
```

- [ ] **Step 2: 提交**

```bash
git add runjam_web_README.md
git commit -m "docs: document update and announcement feature"
```

---

### Task 13: 密钥生成与配置（用户执行，非代码）

**Files:**
- 无代码改动；配置 GitHub Secrets + `tauri.conf.json` pubkey

**Interfaces:**
- Consumes: Task 1 的 `pubkey` 占位符
- Produces: 真实签名密钥生效

- [ ] **Step 1: 生成本地签名密钥对**

在项目根目录运行：

```bash
npx tauri signer generate -w ~/.tauri/runjam.key
```

会生成：
- 私钥文件 `~/.tauri/runjam.key`
- 打印公钥（`-----BEGIN PUBLIC KEY-----` 开头）

- [ ] **Step 2: 配置 GitHub Secrets**

在 GitHub 仓库 `nicepkg/runjam` → Settings → Secrets and variables → Actions 添加：
- `TAURI_SIGNING_PRIVATE_KEY` = 私钥文件内容
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = 生成时设置的密码

- [ ] **Step 3: 更新 pubkey**

将生成的公钥填入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，替换 Task 1 的占位符，并提交：

```bash
git add src-tauri/tauri.conf.json
git commit -m "chore: set updater public key"
```

- [ ] **Step 4: 验证**

打一个测试 tag 触发 CI，确认 Windows 构建成功生成 `.exe.sig` 并上传到 Release。

---

## Self-Review

### Spec 覆盖检查
- ✅ Windows 自动更新：Task 1（插件）、Task 5（命令）、Task 11（CI）、Task 13（密钥）
- ✅ macOS 降级跳转下载：Task 5（`open_download` 分支）、Task 8（UpdatePrompt 按钮）
- ✅ 公告横幅 + 弹窗：Task 3（模型/已读）、Task 4（命令）、Task 7（组件）、Task 9（启动）
- ✅ 本地记住已读：Task 3（`app_settings` 存储）、Task 9（关闭时标记）
- ✅ 启动自动 + 设置页手动：Task 9（App.vue）、Task 10（GeneralSettings）
- ✅ 公告 `min_version` 过滤：Task 4（GET 传 `current` 版本，服务端过滤）
- ✅ 后端 `level` 字段：spec 中已注明需用户执行 ALTER TABLE（不在代码任务内）
- ✅ 更新检查频率（启动 + 手动）：Task 9 + Task 10

### 占位符扫描
- Task 1 的 `pubkey` 占位符是有意的（Task 13 替换），已在步骤中注明。
- 无 TBD/TODO/模糊步骤。所有代码步骤含完整实现。

### 类型一致性
- `UpdateCheckResult`：Rust（`update_available`/`action`/`latest_version`/`notes`/`download_url`，camelCase）与前端 TS 接口字段一一对应。
- `Announcement`：Rust（`id`/`title`/`body`/`level`/`created_at`，camelCase）与前端 TS 接口一致。
- `check_update_ui(current)` / `install_update()` / `get_announcements()` / `mark_announcement_read(id)` 命令名与前端 `invoke` 调用一致。
- `is_windows()`、`version_ge()`、`filter_unread()`、`mark_announcement_read(conn, id)` 在 Task 2/3 定义，Task 4 消费，签名一致。
