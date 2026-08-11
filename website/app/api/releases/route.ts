import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk } from "@/lib/http";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/** 更新日志数据源（供官网 /changelog 页面与客户端共用）。 */
export async function GET() {
  const { data, error } = await supabaseAdmin
    .from("releases")
    .select("version, published_at, notes, download_urls")
    .eq("staged", false)
    .order("published_at", { ascending: false })
    .limit(50);

  if (error) return jsonErr("db error", 500);
  return jsonOk({ releases: data ?? [] });
}
