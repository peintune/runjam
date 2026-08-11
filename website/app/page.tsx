import Link from "next/link";

const features = [
  {
    title: "统一管理多个 Agent",
    desc: "Claude Code、Codex CLI、Gemini CLI 在一个窗口里启动、监控、切换，互不干扰。",
  },
  {
    title: "本地优先",
    desc: "会话数据、成本统计、历史记录全部存在本机 SQLite，你的代码和对话不上云。",
  },
  {
    title: "实时终端与成本统计",
    desc: "内嵌终端实时查看 Agent 输出，token 消耗与费用一目了然。",
  },
  {
    title: "内置技能与工具链",
    desc: "开箱即用的 skills、搜索、代码索引，帮你更快完成任务。",
  },
];

export default function HomePage() {
  return (
    <div>
      <section className="mx-auto max-w-5xl px-4 pb-24 pt-20 text-center">
        <p className="mb-4 inline-block rounded-full border border-amber-500/30 bg-amber-500/10 px-3 py-1 text-xs text-amber-300">
          v0.1.0 · macOS / Windows
        </p>
        <h1 className="mx-auto max-w-3xl text-4xl font-bold leading-tight sm:text-6xl">
          一个窗口，管理你所有的{" "}
          <span className="bg-gradient-to-r from-amber-400 to-orange-500 bg-clip-text text-transparent">
            AI 编程 Agent
          </span>
        </h1>
        <p className="mx-auto mt-6 max-w-2xl text-lg text-zinc-400">
          RunJam 是本地优先的 AI 编程 Agent 桌面管理器：Claude Code、Codex CLI、Gemini CLI
          统一启动、统一监控、统一统计成本。
        </p>
        <div className="mt-10 flex items-center justify-center gap-4">
          <Link
            href="/download"
            className="rounded-lg bg-gradient-to-r from-amber-400 to-orange-500 px-6 py-3 font-semibold text-zinc-950 transition hover:brightness-110"
          >
            免费下载
          </Link>
          <a
            href="https://github.com/nicepkg/runjam"
            target="_blank"
            rel="noreferrer"
            className="rounded-lg border border-zinc-700 px-6 py-3 font-semibold text-zinc-200 transition hover:border-zinc-500"
          >
            GitHub 仓库
          </a>
        </div>
        <div className="mt-16 overflow-hidden rounded-xl border border-zinc-800 bg-zinc-900/60 p-2 shadow-2xl">
          <div className="aspect-video rounded-lg bg-gradient-to-br from-zinc-800 to-zinc-900" />
        </div>
      </section>

      <section className="border-t border-zinc-800/60 bg-zinc-900/40">
        <div className="mx-auto grid max-w-5xl gap-6 px-4 py-16 sm:grid-cols-2">
          {features.map((f) => (
            <div key={f.title} className="rounded-xl border border-zinc-800 bg-zinc-950 p-6">
              <h3 className="text-lg font-semibold text-zinc-100">{f.title}</h3>
              <p className="mt-2 text-sm leading-relaxed text-zinc-400">{f.desc}</p>
            </div>
          ))}
        </div>
      </section>

      <section className="mx-auto max-w-5xl px-4 py-16 text-center">
        <h2 className="text-2xl font-bold">开始使用</h2>
        <p className="mt-3 text-sm text-zinc-400">
          下载安装后，在 RunJam 中点击 + 新建会话，选择你已安装的 CLI Agent 即可。
        </p>
        <Link
          href="/download"
          className="mt-6 inline-block rounded-lg border border-zinc-700 px-6 py-3 font-semibold text-zinc-200 transition hover:border-zinc-500"
        >
          前往下载
        </Link>
      </section>
    </div>
  );
}
