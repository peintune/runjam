# RunJam 软件升级 + 公告 功能设计

- 日期：2026-08-14
- 状态：待评审
- 范围：Windows 自动更新、macOS 降级跳转下载、应用内公告

## 背景与目标

RunJam 是一个 Tauri 2 桌面应用（Vue 3 前端 + Rust 后端）。已有 Vercel + Supabase 后端，
提供 telemetry（register/events/errors）、feedback、updates、announcements、releases 等 API。
安装包发布在 GitHub Releases，由 `build.yml`（tag `v*` 触发）构建 macOS（x64/arm）与 Windows 产物。

当前缺口：
- 前端无任何升级提示 UI（`checkForUpdates` 已封装但无人调用）。
- 未配置 Tauri updater 插件，无法自动下载/安装。
- 公告接口已存在但无前端消费。
- Windows 打包同时产出 MSI 与 NSIS，updater 仅需 NSIS。

目标：
1. **Windows**：完整自动更新（Tauri updater 插件 + NSIS + 签名校验）。
2. **macOS**：降级为"检测新版 → 浏览器跳转官网下载"（无 Apple Developer 账号，无法公证）。
3. **公告**：应用内横幅（普通）+ 弹窗（重要），本地记住已读，只提示一次。
4. 复用现有 Vercel + Supabase 后端与 telemetry 基础设施。

## 平台分流策略（核心）

前端只调用一个统一命令，平台差异封装在 Rust 端。

```
统一命令 check_update_ui()
├── Windows → tauri-plugin-updater 检查，返回可更新状态
└── macOS   → 调现有后端 /api/updates/latest，返回版本 + 下载URL
```

前端拿到统一结构 `UpdateCheckResult`，渲染统一升级提示 UI，按钮行为按平台不同：

| 平台 | action | 行为 |
|---|---|---|
| Windows | `install` | 触发 updater 下载 + 校验 + 安装重启 |
| macOS | `open_download` | 用 opener 打开官网下载页 |

## Windows 自动更新

### 依赖与配置

- `Cargo.toml` 增加 `tauri-plugin-updater = "2"`。
- `lib.rs` 注册插件：`.plugin(tauri_plugin_updater::Builder::new().build())`。
- `tauri.conf.json` 增加：
  ```json
  "plugins": {
    "updater": {
      "pubkey": "<TAURI_SIGNING_PUBLIC_KEY>",
      "endpoints": ["https://runjam-web.vercel.app/api/updates/latest"],
      "windows": { "installMode": "passive" }
    }
  }
  ```
- `capabilities/default.json` 增加 `updater:default`。

### 签名密钥（用户后续执行）

```bash
npx tauri signer generate -w ~/.tauri/runjam.key
```

- 私钥 → GitHub Secret `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
- 公钥 → `tauri.conf.json` 的 `plugins.updater.pubkey`。

### CI 改动（build.yml Windows job）

- 保留 NSIS（`*.exe`），移除 MSI 产物。
- 构建时注入 `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，
  tauri-action 自动为 exe 生成 `.exe.sig` 签名文件。
- 上传产物包含 `*.exe` + `*.exe.sig`（updater 需要）。
- macOS job 不变（仍 ad-hoc 签名 + dmg）。

### 更新检查 + 安装流程（Rust）

```rust
// 新命令（Windows 分支）
async fn check_update_ui(app: AppHandle) -> Result<UpdateCheckResult, String> {
    #[cfg(target_os = "windows")]
    {
        let updater = app.updater().map_err(...)?;
        if let Some(update) = updater.check().await.map_err(...)? {
            return Ok(UpdateCheckResult {
                update_available: true,
                action: "install",
                latest_version: Some(update.version.to_string()),
                notes: update.body.clone(),
                download_url: None,
            });
        }
        return Ok(UpdateCheckResult { update_available: false, ..default });
    }
    #[cfg(not(target_os = "windows"))]
    { /* 走 macOS 降级分支，见下 */ }
}
```

Windows 前端点击"下载并安装"时，调用 `download_and_install` 命令：

```rust
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(...)?;
    if let Some(update) = updater.check().await.map_err(...)? {
        update.download_and_install(|_, _| {}, |_, _| {}).await.map_err(...)?;
    }
    Ok(())
}
```

## macOS 降级（跳转下载）

- 不注册 updater 插件逻辑（`app.updater()` 在非 Windows 平台返回错误，故 macOS 分支用现有 `check_for_updates`）。
- 复用现有 `check_for_updates`（查后端元数据，返回 `downloadUrl`）。
- macOS 分支返回 `action: "open_download"` + `download_url`。
- 前端点击"前往下载" → `tauri-plugin-opener` 打开 `download_url`。

## 公告（Announcements）

### 后端表结构（已确认）

```sql
create table if not exists public.announcements (
  id           uuid primary key default gen_random_uuid(),
  title        text not null,
  body         text not null default '',
  min_version  text,                                -- 仅对 >= 该版本的客户端可见
  active       boolean not null default true,
  level        text not null default 'info',        -- 'info' | 'important'（需后端新增）
  created_at   timestamptz not null default now()
);
```

**后端需新增 `level` 字段**（当前表没有）。用户执行：
```sql
alter table public.announcements
  add column if not exists level text not null default 'info';
```

### 后端接口契约

```
GET /api/announcements?current=<app_version>
→ [{ "id": "<uuid>", "title": "...", "body": "...",
     "level": "info" | "important", "created_at": "..." }]
```

- 服务端过滤 `active = true`。
- 服务端过滤 `min_version`：仅返回 `min_version IS NULL OR current >= min_version` 的公告（版本号比较）。
- 客户端拉取时传当前版本号 `current`。

### 本地已读存储

复用 `app_settings` 表（key-value），key 存 `announcement_read:<id>` = "1"。
无需新建表。

### 拉取与展示流程

```
启动时（App.vue onMounted）
  → GET /api/announcements
  → 过滤掉本地已读的
  → 按 level 分流：
       important → 弹窗 AnnouncementModal
       info      → 横幅 AnnouncementBanner（复用 Toast 容器）
  → 用户关闭时写入已读
```

### 数据模型（Rust）

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub id: String,
    pub title: String,
    pub body: String,
    pub level: String, // "info" | "important"
    pub created_at: Option<String>,
}
```

### 命令

- `get_announcements()` → 拉取后端公告（传当前版本号，服务端已过滤 active/min_version），
  客户端再过滤本地已读，返回未读列表。
- `mark_announcement_read(id)` → 写入 `app_settings`（key = `announcement_read:<id>`）。

## 前端改动

1. `src/api/telemetry.ts`：
   - 新增 `checkUpdateUi()`（返回平台分流结果 `UpdateCheckResult`）。
   - 新增 `installUpdate()`（Windows 触发下载安装）。
   - 新增 `getAnnouncements()`、`markAnnouncementRead(id)`。
2. `src/App.vue`：启动时触发更新检查 + 公告拉取。
3. 新组件：
   - `UpdatePrompt.vue`：升级提示（统一 UI，按钮按平台分流 install/open_download）。
   - `AnnouncementBanner.vue`：右上角可关闭横幅（info）。
   - `AnnouncementModal.vue`：居中弹窗（important），复用遥测弹窗样式。
4. `GeneralSettings.vue`：加"检查更新"按钮 + 当前版本显示。

### 更新检查触发时机

- 启动时自动检查（静默，有新版本才提示）。
- 设置页"检查更新"按钮手动触发。

## 测试策略

### Rust
- `check_update_ui` 平台分流逻辑（Windows vs macOS 返回不同 action）。
- 公告已读去重逻辑（拉取 → 过滤已读 → 标记已读）。

### 前端
- 公告组件渲染（info/important 两种形态）。
- 更新提示按钮行为（install vs open_download）。

### 手动验证
- Windows：构造更高版本号 → 触发 updater → 验证下载、校验、安装。
- macOS：构造更高版本号 → 验证跳转官网下载页。
- 公告：发一条公告 → 启动 → 验证只提示一次。

## 发布流程

`release.sh` 不变。发布后把版本元数据同步到 Supabase `releases` 表（现有流程）。
Windows 的 `.sig` 由 CI 自动生成并随 Release 上传。

## 文件改动清单

**Rust 端**
- `src-tauri/Cargo.toml` — 加 `tauri-plugin-updater`
- `src-tauri/src/lib.rs` — 注册 updater 插件 + 新命令
- `src-tauri/src/commands/telemetry_cmd.rs` — 新增 `check_update_ui`、`install_update`、`get_announcements`、`mark_announcement_read`
- `src-tauri/src/telemetry.rs` — 公告拉取/已读辅助函数（或新建 `updates.rs`）
- `src-tauri/tauri.conf.json` — updater 配置
- `src-tauri/capabilities/default.json` — `updater:default`

**前端**
- `src/api/telemetry.ts` — 新增 API 封装
- `src/App.vue` — 启动检查
- `src/components/UpdatePrompt.vue` — 新
- `src/components/AnnouncementBanner.vue` — 新
- `src/components/AnnouncementModal.vue` — 新
- `src/views/settings/GeneralSettings.vue` — 检查更新入口

**后端（Supabase，用户执行）**
- 公告表加 `level` 字段：`alter table public.announcements add column if not exists level text not null default 'info';`
- `/api/announcements` 接口加 `current` 参数过滤 + 返回 `level` 字段

**CI**
- `.github/workflows/build.yml` — Windows 只出 NSIS + sig，注入签名密钥

**配置/密钥（用户执行）**
- 生成签名密钥对，配置 GitHub Secrets + pubkey
