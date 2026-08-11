import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "RunJam — AI Coding Agent 统一桌面管理",
  description:
    "RunJam 是一个本地优先的 AI 编程 Agent 桌面管理器，在一个窗口里统一管理 Claude Code、Codex CLI、Gemini CLI。",
};

const nav = [
  { href: "/", label: "首页" },
  { href: "/download", label: "下载" },
  { href: "/changelog", label: "更新日志" },
];

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="zh-CN">
      <body className="min-h-screen flex flex-col">
        <header className="sticky top-0 z-50 border-b border-zinc-800/60 bg-zinc-950/80 backdrop-blur">
          <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-4">
            <Link href="/" className="flex items-center gap-2 font-semibold">
              <span className="inline-block h-5 w-5 rounded-md bg-gradient-to-br from-amber-400 to-orange-600" />
              RunJam
            </Link>
            <nav className="flex items-center gap-6 text-sm text-zinc-400">
              {nav.map((n) => (
                <Link key={n.href} href={n.href} className="transition hover:text-zinc-100">
                  {n.label}
                </Link>
              ))}
              <a
                href="https://github.com/nicepkg/runjam"
                target="_blank"
                rel="noreferrer"
                className="rounded-md border border-zinc-700 px-3 py-1.5 text-zinc-300 transition hover:border-zinc-500 hover:text-white"
              >
                GitHub
              </a>
            </nav>
          </div>
        </header>

        <main className="flex-1">{children}</main>

        <footer className="border-t border-zinc-800/60 py-6">
          <div className="mx-auto flex max-w-5xl flex-col items-center gap-2 px-4 text-xs text-zinc-500 sm:flex-row sm:justify-between">
            <span>© {new Date().getFullYear()} RunJam · MIT License</span>
            <span>
              <a href="https://github.com/nicepkg/runjam/issues" className="hover:text-zinc-300">
                反馈问题
              </a>
              {" · "}
              <a href="https://github.com/nicepkg/runjam" className="hover:text-zinc-300">
                GitHub
              </a>
            </span>
          </div>
        </footer>
      </body>
    </html>
  );
}
