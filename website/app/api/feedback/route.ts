import { NextRequest } from "next/server";
import { FeedbackPayload, isUuid, truncate } from "@/lib/telemetry-types";
import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk } from "@/lib/http";

export const runtime = "nodejs";

/** 用户反馈提交。screenshot_url 由客户端先上传到 Supabase Storage 后附带。 */
export async function POST(req: NextRequest) {
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return jsonErr("invalid json");
  }
  const b = body as FeedbackPayload;
  if (!b || typeof b.content !== "string" || b.content.trim().length === 0) {
    return jsonErr("content required");
  }
  if (b.installation_id !== undefined && !isUuid(b.installation_id)) {
    return jsonErr("installation_id invalid");
  }
  const type = ["bug", "feature", "praise", "other"].includes(b.type ?? "")
    ? b.type
    : "other";

  const { data, error } = await supabaseAdmin
    .from("feedback")
    .insert({
      installation_id: b.installation_id ?? null,
      email: truncate(b.email, 254) || null,
      type,
      content: truncate(b.content, 10000),
      screenshot_url: truncate(b.screenshot_url, 500) || null,
      app_version: truncate(b.app_version, 32) || null,
      status: "new",
    })
    .select("id")
    .single();

  if (error) return jsonErr("db error", 500);
  return jsonOk({ id: data?.id }, 201);
}
