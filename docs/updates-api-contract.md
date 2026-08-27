# `/api/updates/latest` 后端接口契约

> 用途：RunJam 升级检测的统一后端接口。
> 同一个 URL 被两个平台用不同的格式消费，后端需按请求特征返回不同 JSON。

- 端点：`https://runjam.app/api/updates/latest`
- 数据源：Supabase `releases` 表（发布后需同步版本元数据，含各平台下载 URL 与 Windows 签名）
- 文件托管：GitHub Releases（安装包文件本身）

---

## 1. 两个消费者，两种格式

| 消费者 | 请求特征 | 期望返回 |
|---|---|---|
| **Windows**（tauri-plugin-updater） | `GET /api/updates/latest`（**无** query 参数） | `latest.json` 格式 |
| **macOS**（`check_for_updates`） | `GET /api/updates/latest?platform=darwin&arch=x86_64&current=1.0.23` | `UpdateInfo` 格式 |

**后端分流规则：**
- 请求带 `platform` query 参数 → 返回 **`UpdateInfo`**（macOS）
- 请求不带 `platform` 参数 → 返回 **`latest.json`**（Windows updater）

---

## 2. Windows 期望的 `latest.json` 格式

```json
{
  "version": "1.0.24",
  "notes": "Release notes 文本",
  "pub_date": "2026-08-14T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEZDMzVBNEMzMzYwRjFCREIK...",
      "url": "https://github.com/peintune/runjam/releases/download/v1.0.24/RunJam_1.0.24_x64-setup.exe"
    }
  }
}
```

字段说明：
- `version`：新版本号，必须 ≥ 当前版本才触发更新。
- `notes`：展示给用户的更新说明。
- `pub_date`：RFC3339 时间戳。
- `platforms`：键是 **Tauri target 三元组**，值含：
  - `signature`：**`.exe.sig` 文件的完整内容**（base64 字符串）。
  - `url`：`.exe`（NSIS 安装包）的下载地址。

### 关键：`signature` 从哪来

`signature` = GitHub Release 里 `.exe.sig` 文件的文本内容。

- 发布时 CI 用 `TAURI_SIGNING_PRIVATE_KEY` 生成 `.exe.sig`。
- 后端同步 `releases` 表时，需把该 `.sig` 的**内容**存进数据库，返回时放进 `signature` 字段。
- `.sig` 内容与 `tauri.conf.json` 里的 `plugins.updater.pubkey` 必须**来自同一对密钥**，否则 updater 校验失败、拒绝安装。

---

## 3. macOS 期望的 `UpdateInfo` 格式

```json
{
  "update_available": true,
  "latest_version": "1.0.24",
  "published_at": "2026-08-14T12:00:00Z",
  "notes": "Release notes 文本",
  "download_url": "https://github.com/peintune/runjam/releases/download/v1.0.24/RunJam_1.0.24_aarch64.dmg",
  "download_urls": {
    "x86_64": "https://github.com/peintune/runjam/releases/download/v1.0.24/RunJam_1.0.24_x64.dmg",
    "aarch64": "https://github.com/peintune/runjam/releases/download/v1.0.24/RunJam_1.0.24_aarch64.dmg"
  }
}
```

字段说明（注意是 **snake_case**，Rust 端 `UpdateInfo` 未加 `rename_all`）：
- `update_available`：是否可更新（后端按 `current` 版本比较）。
- `latest_version`：最新版本号。
- `published_at`：发布时间。
- `notes`：更新说明。
- `download_url`：**当前请求平台/架构**对应的 `.dmg` 下载地址（macOS 跳转下载用）。
- `download_urls`：按架构映射的下载地址（可选，供前端按需选择）。

### macOS 的 `download_url` 选择逻辑

macOS 请求带 `platform=darwin&arch=x86_64`（或 `aarch64`），后端应：
- 按 `arch` 返回对应架构的 `.dmg` 到 `download_url`。

---

## 4. 目标平台 key 映射

| Tauri target | `platforms` key（latest.json） | macOS `arch` 参数 |
|---|---|---|
| Windows x64 | `windows-x86_64` | — |
| macOS Intel | `darwin-x86_64` | `x86_64` |
| macOS Apple Silicon | `darwin-aarch64` | `aarch64` |
| Linux x64 | `linux-x86_64` | `x86_64` |

> 注：`platform_name()` 在 Rust 端把 `macos` 映射为 `darwin`、`windows` 映射为 `win32`，但 **latest.json 的 platforms 键用的是 tauri 的 target 三元组**（`darwin-x86_64` 等），两者不同，后端别混淆。

---

## 5. 建议的 `releases` 表结构（Supabase）

```sql
create table if not exists public.releases (
  id            uuid primary key default gen_random_uuid(),
  version       text not null,                 -- e.g. '1.0.24'
  notes         text,
  published_at  timestamptz not null default now(),
  staged        boolean not null default false, -- staged=true 时才对外可见
  download_urls jsonb,                          -- { "x86_64": "...", "aarch64": "..." }
  windows_sig   text,                           -- .exe.sig 文件内容
  windows_url   text                            -- .exe 下载地址
);
```

---

## 6. 发布后同步流程

1. `release.sh` 打 tag → GitHub Actions 构建并发布 Release（含 `.exe` + `.exe.sig` + `.dmg`）。
2. 把版本元数据插入/更新到 Supabase `releases` 表：
   - `version`、`notes`、`published_at`
   - `download_urls`（macOS 各架构 dmg）
   - `windows_sig`（从 Release 下载 `.exe.sig` 的内容）
   - `windows_url`（`.exe` 下载地址）
3. `staged = true` 后，客户端即可检测到更新。

---

## 7. 常见坑

- **`.sig` 缺失**：Windows 升级会失败（校验签名失败）。确保 CI 生成并上传 `.exe.sig`，且后端 `windows_sig` 存的是文件**内容**。
- **公钥不匹配**：`tauri.conf.json` 的 `pubkey` 与签名私钥必须同源，否则 updater 拒绝。
- **版本号比较**：后端用 semver 比较 `current` 与 `latest_version`，避免字符串比较出错（如 `1.0.10` > `1.0.9`）。
- **格式混淆**：别把 `latest.json` 的 `platforms` 键（target 三元组）和 macOS 的 `arch` 参数混用。
