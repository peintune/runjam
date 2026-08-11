import { NextRequest } from "next/server";
import { isUuid, truncate } from "@/lib/telemetry-types";
import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk } from "@/lib/http";
import { clientCountry } from "@/lib/telemetry-types";

export const runtime = "nodejs";

/**
 * 首次启动注册设备（幂等）。
 * IP 不上报：客户端不传 IP，服务端只从请求头解析"国家"，不落库原始 IP。
 */
export async function POST(req: NextRequest) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return jsonErr("invalid json");
  }
  const b = body as Record<string, unknown>;
  if (!b || !isUuid(b.installation_id)) return jsonErr("installation_id (uuid) required");

  const country = clientCountry(req.headers);

  const { error } = await supabaseAdmin.from("devices").upsert(
    {
      installation_id: b.installation_id,
      app_version: truncate(b.app_version, 32) || "unknown",
      platform: truncate(b.platform, 16) || "unknown",
      arch: truncate(b.arch, 16) || "unknown",
      os_version: truncate(b.os_version, 64) || null,
      telemetry_enabled: b.telemetry_enabled !== false,
      ip_country: country,
      first_seen_at: new Date().toISOString(),
      last_seen_at: new Date().toISOString(),
    },
    { onConflict: "installation_id" }
  );

  if (error) return jsonErr("db error", 500);
  return jsonOk({ installation_id: b.installation_id });
}
