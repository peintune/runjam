# RunJam 官网 + 后台 API

RunJam 官方站点与遥测/反馈/更新 API。部署在 **Vercel**，数据在 **Supabase**，安装包发布在 **GitHub Releases**。

## 技术栈

- Next.js 15 (App Router) + React 19 + Tailwind CSS 4 + TypeScript
- `@supabase/supabase-js`（服务端用 service_role key 写入）
- 一个 Vercel 项目 = 官网页面 + `/api/*` 接口，桌面端统一访问 `https://<domain>/api/*`

## 页面

| 路径 | 说明 |
|---|---|
| `/` | 首页（产品介绍 + 下载 CTA） |
| `/download` | 下载页，按平台分流，版本号实时取自 `/api/updates/latest` |
| `/changelog` | 更新日志，数据取自 Supabase `releases` 表 |

## API

| 接口 | 方法 | 用途 |
|---|---|---|
| `/api/telemetry/register` | POST | 设备注册（幂等），服务端从请求 IP 解析国家 |
| `/api/telemetry/events` | POST | 批量功能埋点（1~100 条） |
| `/api/telemetry/errors` | POST | 批量错误日志（客户端已脱敏） |
| `/api/feedback` | POST | 用户反馈提交 |
| `/api/updates/latest` | GET | 客户端更新检查，返回最新版本 + 下载直链 |
| `/api/announcements` | GET | 应用内公告 |
| `/api/releases` | GET | 更新日志列表 |
| `/api/health` | GET | 健康检查 |

## 本地开发

```bash
npm install
cp .env.example .env.local   # 填入 Supabase 项目 URL / anon key / service_role key
npm run dev                  # http://localhost:3000
```

## 部署到 Vercel

1. 在 [vercel.com](https://vercel.com) 导入本目录（或推送到独立 GitHub 仓库后导入）。
2. 项目设置 → Environment Variables 添加：
   - `NEXT_PUBLIC_SUPABASE_URL`
   - `NEXT_PUBLIC_SUPABASE_ANON_KEY`
   - `SUPABASE_SERVICE_ROLE_KEY`
3. Deploy。框架自动识别为 Next.js，无需额外配置。

## 初始化数据库

在 Supabase Dashboard → SQL Editor 中执行 `supabase/schema.sql`（建表 + RLS + 视图）。

> 也可以在本目录执行 `npx supabase db push`（需先在 Supabase 里初始化并链接）。

## 发布新版本（GitHub Releases → 官网可见）

打包发布仍走桌面仓库的 `build.yml`（`git tag v*` 触发，产物上传 GitHub Release）。
发布后把版本元数据同步到 Supabase `releases` 表（二选一）：

- 手动：SQL Editor 执行
  ```sql
  insert into releases (version, published_at, notes, download_urls, staged)
  values ('v0.2.0', now(), '# 更新内容', '{"macos_aarch64":"https://github.com/peintune/runjam/releases/download/v0.2.0/xxx.dmg"}', false);
  ```
- 自动（推荐）：在桌面仓库 CI 里加一步调用
  `POST https://<your-domain>/api/…`（或直接用 GitHub Actions 里的 supabase-js / psql 更新表）。

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

## 灰度发布

`releases.staged = true` 的版本不会出现在 `/api/updates/latest` 与官网更新日志中，
可用于内测渠道（内测客户端直接请求 staged 版本接口，按安装白名单放行）。

## 隐私说明

- 客户端不上报 IP；服务端只解析「国家」存 `devices.ip_country`，原始 IP 不落库。
- 错误日志在客户端脱敏（去本地路径 / 环境变量 / token）。
- 遥测默认开启，首次启动弹窗说明，设置页可一键关闭（见桌面端 README）。
