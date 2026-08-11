"use client";

import { useEffect, useState } from "react";

type LatestRelease = {
  version: string;
  published_at: string | null;
  notes: string;
  download_urls: Record<string, string> | null;
};

const platforms = [
  {
    key: "macos_aarch64",
    name: "macOS（Apple Silicon）",
    hint: "M1 / M2 / M3 / M4",
    asset: "macos_aarch64",
  },
  {
    key: "macos_x86_64",
    name: "macOS（Intel）",
    hint: "x64",
    asset: "macos_x86_64",
  },
  { key: "windows_x64", name: "Windows", hint: "x64 · MSI / NSIS", asset: "windows_x64" },
  { key: "linux_x64", name: "Linux", hint: "x64（即将支持）", asset: "linux_x64", soon: true },
];

const GITHUB_LATEST = "https://github.com/nicepkg/runjam/releases/latest";

export default function DownloadPage() {
  const [latest, setLatest] = useState<LatestRelease | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch("/api/updates/latest")
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(setLatest)
      .catch(() => setError("获取版本信息失败，请前往 GitHub Releases 下载。"));
  }, []);

  return (
    <div className="mx-auto max-w-3xl px-4 py-16">
      <h1 className="text-3xl font-bold">下载 RunJam</h1>
      <p className="mt-3 text-sm text-zinc-400">
        当前最新版本：
        {latest ? (
          <span className="font-mono text-zinc-200">{latest.version}</span>
        ) : (
          <span className="text-zinc-500">{error ?? "获取中…"}</span>
        )}
        {latest?.published_at && (
          <span className="ml-2">发布于 {latest.published_at.slice(0, 10)}</span>
        )}
      </p>

      <div className="mt-8 grid gap-4 sm:grid-cols-2">
        {platforms.map((p) => {
          const url =
            (latest?.download_urls && latest.download_urls[p.asset]) || GITHUB_LATEST;
          return (
            <a
              key={p.key}
              href={url}
              target="_blank"
              rel="noreferrer"
              aria-disabled={!!p.soon}
              className={`rounded-xl border border-zinc-800 bg-zinc-900/60 p-6 transition ${
                p.soon ? "pointer-events-none opacity-50" : "hover:border-zinc-600"
              }`}
            >
              <h3 className="font-semibold">{p.name}</h3>
              <p className="mt-1 text-xs text-zinc-500">{p.hint}</p>
              {!p.soon && (
                <span className="mt-4 inline-block text-sm text-amber-400">下载 →</span>
              )}
            </a>
          );
        })}
      </div>

      <p className="mt-8 text-xs leading-relaxed text-zinc-500">
        所有安装包均发布在 GitHub Releases：
        <a href={GITHUB_LATEST} target="_blank" rel="noreferrer" className="text-zinc-300 underline">
          github.com/nicepkg/runjam/releases
        </a>
        。macOS 首次打开如提示「无法验证开发者」，请在系统设置 → 隐私与安全性中允许。
      </p>
    </div>
  );
}
