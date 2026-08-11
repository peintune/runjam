import { NextRequest } from "next/server";
import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk } from "@/lib/http";
import { versionGreaterThan } from "@/lib/http";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** 应用内公告：GET /api/announcements?version=0.1.0（只返回对当前版本有效的公告） */
export async function GET(req: NextRequest) {
  const version = req.nextUrl.searchParams.get("version") ?? "";

  const { data, error } = await supabaseAdmin
    .from("announcements")
    .select("id, title, body, min_version, created_at")
    .eq("active", true)
    .order("created_at", { ascending: false })
    .limit(20);

  if (error) return jsonErr("db error", 500);

  const visible = (data ?? []).filter(
    (a: { min_version: string | null }) =>
      !a.min_version || version === "" || versionGreaterThan(version, a.min_version)
  );

  return jsonOk({
    announcements: visible.map((a: Record<string, unknown>) => ({
      id: a.id,
      title: a.title,
      body: a.body,
      created_at: a.created_at,
    })),
  });
}
