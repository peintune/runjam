-- ============================================================
-- RunJam 后台数据库 Schema（在 Supabase Dashboard → SQL Editor 执行）
-- 表 + RLS + 报表视图
-- ============================================================

-- ── 设备表（每台安装注册一次，幂等）────────────────────────
create table if not exists public.devices (
  installation_id  uuid primary key,
  app_version      text not null default 'unknown',
  platform         text not null default 'unknown',  -- darwin / win32 / linux
  arch             text not null default 'unknown',  -- x64 / arm64
  os_version       text,
  telemetry_enabled boolean not null default true,
  ip_country       text,                             -- 仅国家（ISO 3166-1 alpha-2），不存原始 IP
  first_seen_at    timestamptz not null default now(),
  last_seen_at     timestamptz not null default now()
);

-- ── 功能埋点（批量上报）───────────────────────────────────
create table if not exists public.events (
  id               bigint generated always as identity primary key,
  installation_id  uuid not null references public.devices(installation_id) on delete cascade,
  event_name       text not null,                    -- 如 session_start / tool_call
  event_props      jsonb,
  app_version      text,
  platform         text,
  event_time       timestamptz not null default now()
);
create index if not exists idx_events_time      on public.events (event_time desc);
create index if not exists idx_events_name_time on public.events (event_name, event_time desc);

-- ── 错误日志（客户端脱敏后上报）────────────────────────────
create table if not exists public.error_logs (
  id               bigint generated always as identity primary key,
  installation_id  uuid not null references public.devices(installation_id) on delete cascade,
  level            text not null default 'error',   -- error / warn / info
  category         text not null default 'unknown', -- rust_panic / js_error / acp_error ...
  message          text not null,
  stack            text,                            -- 已截断（8KB）
  context          jsonb,
  created_at       timestamptz not null default now()
);
create index if not exists idx_error_logs_time on public.error_logs (created_at desc);

-- ── 用户反馈 ──────────────────────────────────────────────
create table if not exists public.feedback (
  id               uuid primary key default gen_random_uuid(),
  installation_id  uuid references public.devices(installation_id) on delete set null,
  email            text,
  type             text not null default 'other',   -- bug / feature / praise / other
  content          text not null,
  screenshot_url   text,
  app_version      text,
  status           text not null default 'new',     -- new / triaged / done / wontfix
  created_at       timestamptz not null default now()
);
create index if not exists idx_feedback_status_time on public.feedback (status, created_at desc);

-- ── GitHub Release 元数据镜像 ─────────────────────────────
create table if not exists public.releases (
  version        text primary key,                  -- 如 v0.2.0
  published_at   timestamptz not null default now(),
  github_url     text,
  notes          text not null default '',          -- changelog markdown
  download_urls  jsonb,                             -- {macos_aarch64, macos_x86_64, windows_x64, linux_x64}
  staged         boolean not null default false     -- true = 灰度，不出现在公开接口
);

-- ── 应用内公告 ────────────────────────────────────────────
create table if not exists public.announcements (
  id           uuid primary key default gen_random_uuid(),
  title        text not null,
  body         text not null default '',
  min_version  text,                                -- 仅对 >= 该版本的客户端可见
  active       boolean not null default true,
  created_at   timestamptz not null default now()
);

-- ============================================================
-- RLS：服务端走 service_role（自动绕过）；anon 只允许必要操作
-- ============================================================
alter table public.devices      enable row level security;
alter table public.events       enable row level security;
alter table public.error_logs   enable row level security;
alter table public.feedback     enable row level security;
alter table public.releases     enable row level security;
alter table public.announcements enable row level security;

-- 遥测/反馈表：匿名只允许 insert（防止公开读数据），写前服务端做校验
drop policy if exists "anon can insert devices"    on public.devices;
create policy "anon can insert devices"    on public.devices    for insert to anon with check (true);
drop policy if exists "anon can insert events"     on public.events;
create policy "anon can insert events"     on public.events     for insert to anon with check (true);
drop policy if exists "anon can insert error_logs" on public.error_logs;
create policy "anon can insert error_logs" on public.error_logs for insert to anon with check (true);
drop policy if exists "anon can insert feedback"   on public.feedback;
create policy "anon can insert feedback"   on public.feedback   for insert to anon with check (true);

-- 公开只读：releases / announcements（供官网 SSR 与客户端读取）
drop policy if exists "public read releases"       on public.releases;
create policy "public read releases"       on public.releases      for select to anon using (staged = false);
drop policy if exists "public read announcements"  on public.announcements;
create policy "public read announcements"  on public.announcements for select to anon using (active = true);

-- ============================================================
-- 报表视图（Supabase Dashboard → SQL 可直接查询，或导出 CSV）
-- ============================================================

-- 每日活跃设备（DAU）：当天有事件上报的设备数
create or replace view public.v_dau_daily as
select
  date_trunc('day', e.event_time)::date as day,
  count(distinct e.installation_id)      as dau
from public.events e
group by 1
order by 1 desc;

-- 版本分布
create or replace view public.v_version_distribution as
select app_version, count(*) as devices, max(last_seen_at) as last_seen_at
from public.devices
group by 1
order by 2 desc;

-- 平台/国家分布
create or replace view public.v_platform_country as
select platform, ip_country, count(*) as devices
from public.devices
group by 1, 2
order by 3 desc;

-- 功能使用 Top N（近 30 天）
create or replace view public.v_top_events as
select event_name, count(*) as cnt
from public.events
where event_time > now() - interval '30 days'
group by 1
order by 2 desc;

-- 每日崩溃率：错误日志条数 / 当日活跃设备
create or replace view public.v_crash_daily as
select
  day,
  errors,
  dau,
  case when dau > 0 then round(errors::numeric / dau, 3) else 0 end as errors_per_active_device
from (
  select
    date_trunc('day', t.created_at)::date as day,
    count(*)                        as errors,
    coalesce(d.dau, 0)              as dau
  from public.error_logs t
  left join public.v_dau_daily d on d.day = date_trunc('day', t.created_at)::date
  group by 1, d.dau
) s
order by 1 desc;

-- 反馈状态流转
create or replace view public.v_feedback_status as
select status, type, count(*) as cnt, max(created_at) as last_at
from public.feedback
group by 1, 2
order by 3 desc;
