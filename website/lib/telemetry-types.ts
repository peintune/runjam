/** 与客户端（src-tauri telemetry.rs / 前端）约定的上报数据结构。 */

export type RegisterPayload = {
  installation_id: string;
  app_version: string;
  platform: string; // darwin / win32 / linux
  arch: string; // x64 / arm64
  os_version?: string;
  telemetry_enabled?: boolean;
};

export type TelemetryEvent = {
  event_name: string;
  event_props?: Record<string, unknown>;
  event_time?: string; // ISO 8601
};

export type EventsPayload = {
  installation_id: string;
  events: TelemetryEvent[];
};

export type ErrorLogEntry = {
  level?: "error" | "warn" | "info";
  category?: string; // rust_panic / js_error / acp_error / ...
  message: string;
  stack?: string;
  context?: Record<string, unknown>;
  created_at?: string;
};

export type ErrorsPayload = {
  installation_id: string;
  errors: ErrorLogEntry[];
};

export type FeedbackPayload = {
  installation_id?: string;
  email?: string;
  type: "bug" | "feature" | "praise" | "other";
  content: string;
  screenshot_url?: string;
  app_version?: string;
};

export const MAX_BATCH = 100;
export const MAX_STRING = 4000;

/** 截断字符串，防脏数据。 */
export function truncate(s: unknown, max = MAX_STRING): string {
  if (typeof s !== "string") return "";
  return s.length > max ? s.slice(0, max) : s;
}

export function isUuid(s: unknown): s is string {
  return (
    typeof s === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s)
  );
}

export function safeJson(v: unknown, max = 64 * 1024): Record<string, unknown> | null {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return null;
  try {
    const s = JSON.stringify(v);
    return s.length > max ? null : (v as Record<string, unknown>);
  } catch {
    return null;
  }
}

/** 客户端 IP 国家（ISO 3166-1 alpha-2）。IP 本身不落库，规避 PII。 */
export function clientCountry(headers: Headers): string | null {
  const c = headers.get("x-vercel-ip-country");
  return c && /^[A-Za-z]{2}$/.test(c) ? c.toUpperCase() : null;
}
