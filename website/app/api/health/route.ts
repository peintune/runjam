import { jsonOk } from "@/lib/http";
import { isSupabaseConfigured } from "@/lib/supabase";

export const runtime = "nodejs";

export async function GET() {
  return jsonOk({ status: "ok", supabase: isSupabaseConfigured });
}
