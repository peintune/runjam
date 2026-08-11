import { NextRequest } from "next/server";
import {
  ErrorsPayload,
  MAX_BATCH,
  isUuid,
  safeJson,
  truncate,
} from "@/lib/telemetry-types";
import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk } from "@/lib/http";

export const runtime = "nodejs";

/** 批量错误日志上报（客户端已做脱敏：去本地路径 / 环境变量 / token）。 */
export async function POST(req: NextRequest) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return jsonErr("invalid json");
  }
  const b = body as ErrorsPayload;
  if (!b || !isUuid(b.installation_id)) return jsonErr("installation_id (uuid) required");
  if (!Array.isArray(b.errors) || b.errors.length === 0 || b.errors.length > MAX_BATCH) {
    return jsonErr(`errors required (1..${MAX_BATCH})`);
  }

  const rows = b.errors.slice(0, MAX_BATCH).map((e) => ({
    installation_id: b.installation_id,
    level: ["error", "warn", "info"].includes(e.level ?? "") ? e.level : "error",
    category: truncate(e.category, 32) || "unknown",
    message: truncate(e.message, 2000),
    stack: truncate(e.stack, 8000) || null,
    context: safeJson(e.context, 16 * 1024),
    created_at: e.created_at ?? new Date().toISOString(),
  }));

  const { error } = await supabaseAdmin.from("error_logs").insert(rows);
  if (error) return jsonErr("db error", 500);
  return jsonOk({ inserted: rows.length });
}
