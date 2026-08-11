import { NextRequest } from "next/server";
import { supabaseAdmin } from "@/lib/supabase";
import { jsonErr, jsonOk, versionGreaterThan } from "@/lib/http";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * 客户端更新检查：GET /api/updates/latest?platform=&arch=&current=
 * - current 为客户端当前版本（如 "0.1.0"）
 * - staged 的灰度版本不在此接口返回（灰度通过单独的 staged 通道控制）
 * - 安装包本体在 GitHub Releases，这里只返回元数据 + 直链
 */
export async function GET(req: NextRequest) {
  const sp = req.nextUrl.searchParams;
  const current = sp.get("current") ?? "";
  const platform = sp.get("platform") ?? "";
  const arch = sp.get("arch") ?? "";

  const { data, error } = await supabaseAdmin
    .from("releases")
    .select("version, published_at, notes, download_urls")
    .eq("staged", false)
    .order("published_at", { ascending: false })
    .limit(50);

  if (error) return jsonErr("db error", 500);
  if (!data || data.length === 0) {
    return jsonOk({ update_available: false, latest_version: null, download_urls: {} });
  }

  // 取最高版本
  const latest = data.reduce((a, b) =>
    versionGreaterThan(a.version, b.version) ? a : b
  );

  // 下载 URL：优先 releases 表里的直链；缺失时回退 GitHub latest。
  const urls = (latest.download_urls as Record<string, string> | null) ?? {};
  const platformKey = `${platform}_${arch}`;
  const fallbackKey = platform === "darwin" ? "macos_aarch64" : `${platform}_x64`;
  const downloadUrl =
    urls[platformKey] ??
    urls[fallbackKey] ??
    "https://github.com/nicepkg/runjam/releases/latest";

  const updateAvailable = current !== "" && versionGreaterThan(latest.version, current);

  return jsonOk({
    update_available: updateAvailable,
    latest_version: latest.version,
    published_at: latest.published_at,
    notes: latest.notes ?? "",
    download_url: downloadUrl,
    download_urls: urls,
  });
}
