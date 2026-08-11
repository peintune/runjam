import { createClient } from "@supabase/supabase-js";

/**
 * Supabase 客户端封装。
 *
 * - supabaseAnon：使用 anon key，走 RLS，适合服务端渲染的公开只读（如 releases 表）。
 * - supabaseAdmin：使用 service_role key，仅存在于 Vercel 服务端环境变量，
 *   用于写入 telemetry / feedback 等（RLS 对 service_role 默认放行）。
 */

const url = process.env.NEXT_PUBLIC_SUPABASE_URL ?? "";
const anonKey = process.env.NEXT_PUBLIC_SUPABASE_ANON_KEY ?? "";
const serviceKey = process.env.SUPABASE_SERVICE_ROLE_KEY ?? "";

export const isSupabaseConfigured = Boolean(url && anonKey && serviceKey);

// 未配置时使用占位值，避免 createClient 在模块加载时抛错；请求会失败并由
// 各路由/页面兜底（/api/health 会如实报告 supabase: false）。
const baseUrl = url || "https://placeholder.supabase.co";
const anonFallback = anonKey || "anon-placeholder-key";

export const supabaseAnon = createClient(baseUrl, anonFallback, {
  auth: { persistSession: false },
});

export const supabaseAdmin = createClient(baseUrl, serviceKey || anonFallback, {
  auth: { persistSession: false },
});
