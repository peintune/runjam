import { NextRequest } from "next/server";
import {
  EventsPayload,
  MAX_BATCH,
  isUuid,
  safeJson,
  truncate,
} from "@/lib/telemetry-types";
import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk } from "@/lib/http";

export const runtime = "nodejs";
export const maxDuration = 30;

/** 批量功能埋点（客户端本地队列，10~100 条一批）。 */
export async function POST(req: NextRequest) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return jsonErr("invalid json");
  }
  const b = body as EventsPayload;
  if (!b || !isUuid(b.installation_id)) return jsonErr("installation_id (uuid) required");
  if (!Array.isArray(b.events) || b.events.length === 0 || b.events.length > MAX_BATCH) {
    return jsonErr(`events required (1..${MAX_BATCH})`);
  }

  const rows = b.events.slice(0, MAX_BATCH).map((e) => ({
    installation_id: b.installation_id,
    event_name: truncate(e.event_name, 64),
    event_props: safeJson(e.event_props, 16 * 1024),
    app_version: null,
    platform: null,
    event_time: e.event_time ?? new Date().toISOString(),
  }));

  const { error } = await supabaseAdmin.from("events").insert(rows);
  if (error) return jsonErr("db error", 500);
  return jsonOk({ inserted: rows.length });
}
