import { supabaseAdmin } from "@/lib/supabase";

export const dynamic = "force-dynamic";

type Release = {
  version: string;
  published_at: string | null;
  notes: string;
  staged: boolean;
};

export default async function ChangelogPage() {
  let releases: Release[] = [];
  let error: string | null = null;

  try {
    const { data, error: e } = await supabaseAdmin
      .from("releases")
      .select("version, published_at, notes, staged")
      .eq("staged", false)
      .order("published_at", { ascending: false })
      .limit(20);
    if (e) throw e;
    releases = (data ?? []) as Release[];
  } catch {
    error = "加载失败，请稍后再试。";
  }

  return (
    <div className="mx-auto max-w-3xl px-4 py-16">
      <h1 className="text-3xl font-bold">更新日志</h1>
      <p className="mt-3 text-sm text-zinc-400">
        完整发布历史见 GitHub Releases。安装包更新请使用应用内的自动升级。
      </p>

      {error && <p className="mt-6 text-sm text-red-400">{error}</p>}

      <div className="mt-8 space-y-8">
        {releases.map((r) => (
          <article key={r.version} className="border-l-2 border-zinc-800 pl-6">
            <h2 className="font-mono text-lg font-semibold text-amber-400">{r.version}</h2>
            <p className="mt-1 text-xs text-zinc-500">
              {r.published_at ? new Date(r.published_at).toLocaleDateString("zh-CN") : ""}
            </p>
            <pre className="mt-3 whitespace-pre-wrap font-sans text-sm leading-relaxed text-zinc-300">
              {r.notes}
            </pre>
          </article>
        ))}
        {!error && releases.length === 0 && (
          <p className="text-sm text-zinc-500">暂无发布记录。</p>
        )}
      </div>
    </div>
  );
}
