import type { Locale } from "./en";

const zh: Locale = {
  nav: {
    features: "功能",
    architecture: "架构",
    compare: "对比",
    faq: "FAQ",
    github: "GitHub",
    language: "English",
  },
  hero: {
    badge: "开源 · 本地优先 · MIT 协议",
    title1: "一个桌面，",
    title2: "你的所有 AI Agent",
    title3: "真正地协同工作。",
    subtitle:
      "RunJam 像 Docker Desktop 管理容器一样管理你的 AI Coding Agent — 自动检测、一键安装、统一对话界面，让 <strong>Claude Code</strong>、<strong>Codex CLI</strong>、<strong>Gemini CLI</strong> 在同一处协作。",
    download: "下载 macOS",
    viewSource: "查看源码",
  },
  terminal: {
    prompt: '"帮我写一个 Rust TCP 服务器"',
    running: "Claude Code running...",
    thinking: "Thinking: 分析需求、规划结构、考虑异常处理...",
    tool: "Tool: write_to_file → src/server.rs",
    done: "Done · 完成了 TCP 服务器的实现，支持并发连接和优雅关闭",
  },
  features: {
    title: "比想象中更简单",
    subtitle: "无需改造 Agent、无需复杂配置。你的 AI Agent，就应该这样管理。",
    cards: [
      {
        title: "零配置，开箱即用",
        desc: "自动检测你系统中已安装的 AI Agent，无需手动配置路径或参数。像 Docker Desktop 管理容器一样简单。",
      },
      {
        title: "多 Agent 统一管理",
        desc: "Claude Code、Codex CLI、Gemini CLI 在同一界面管理。切换 Agent 就像切换聊天对象一样自然。",
      },
      {
        title: "实时流式输出",
        desc: "Agent 的思考过程、工具调用、最终回复全部实时流式展示，支持 Markdown 渲染和代码高亮。",
      },
      {
        title: "本地优先，数据安全",
        desc: "所有对话数据存储在本地，Agent 进程在你的机器上运行。不收集遥测数据，不依赖云端服务。",
      },
      {
        title: "统一模型配置",
        desc: "一次配置，所有 Agent 同步使用。支持 OpenAI、Anthropic、Google、DeepSeek 等主流厂商。",
      },
      {
        title: "跨平台桌面应用",
        desc: "基于 Tauri 构建（Rust + Vue3），macOS / Windows / Linux 全平台支持，体积小，性能好。",
      },
    ],
  },
  agents: {
    title: "最好的 AI 代理们，在同一处",
    subtitle: "RunJam 自动检测、一键安装。无需手动配置路径或环境变量。",
    steps: ["扫描 PATH 环境变量", "自动检测已安装 Agent", "未安装？一键安装", "准备就绪！"],
  },
  architecture: {
    title: "为性能而生",
    subtitle: "Tauri + Rust + Vue 3，轻量、快速、安全。",
    cards: [
      {
        title: "Rust 内核",
        desc: "Agent 进程管理、stdout 解析、文件系统操作全部由 Rust 处理。零 GC 停顿，内存占用 < 50MB。",
      },
      {
        title: "Vue 3 前端",
        desc: "响应式 UI + Tailwind CSS v4 + TypeScript。流畅的 Markdown 渲染、代码高亮、流式打字机效果。",
      },
      {
        title: "Tauri 框架",
        desc: "比 Electron 小 90% 的安装包。原生系统 API 调用，macOS / Windows / Linux 全平台一致体验。",
      },
    ],
    diagram: {
      frontend: "Frontend",
      frontendTech: "Vue 3 + TS",
      frontendComp: "ChatMessages",
      core: "Core",
      coreTech: "Rust Runtime",
      coreComp: "Session Manager",
      agent: "Agent",
      agentTech: "CLI Process",
      agentComp: "stdin/stdout pipe",
      ipc: "Tauri IPC ↔ Rust Backend ↔ Child Process stdin/stdout",
    },
  },
  highlights: {
    chat: {
      label: "统一对话界面",
      title: "像聊天一样<br>与多个 Agent 交互",
      desc: "每条消息按项目组织，Agent 类型标识一目了然。支持 Markdown 渲染、代码高亮、Mermaid 图表、思考过程实时展示。",
      bullets: [
        "实时流式输出 + 打字机效果",
        "思考过程分开展示，自动折叠",
        "工具调用明细，展开查看输入输出",
        "代码块高亮 + 一键复制",
      ],
    },
    model: {
      label: "统一模型中心",
      title: "配置一次模型<br>所有 Agent 自动同步",
      desc: "在 RunJam 中添加模型配置，自动同步到 Claude Code、Codex CLI、Gemini CLI 的配置文件中。支持 OpenAI、Anthropic、Google、Groq、DeepSeek 及自定义 API。",
      providers: ["OpenAI", "Anthropic", "Google AI", "Groq", "DeepSeek"],
      custom: "+ 自定义",
    },
  },
  compare: {
    title: "为什么选择 RunJam",
    subtitle: "无需改造 Agent、无需 ACP 协议。原生 CLI 即可工作。",
    tableHead: {
      feature: "功能",
    },
    rows: [
      {
        feature: "Agent 改造要求",
        runjam: "无需改造",
        aionui: "需实现 ACP 协议",
        cursor: "N/A",
      },
      {
        feature: "多 Agent 管理",
        runjam: "统一界面",
        aionui: "统一界面",
        cursor: "单一 Agent",
      },
      {
        feature: "本地数据存储",
        runjam: "完全本地",
        aionui: "完全本地",
        cursor: "云端同步",
      },
      {
        feature: "模型统一配置",
        runjam: "一次配置全部同步",
        aionui: "per-backend 配置",
        cursor: "内置模型",
      },
      {
        feature: "桌面框架",
        runjam: "Tauri (Rust)",
        aionui: "Electron (Node)",
        cursor: "Electron",
      },
      {
        feature: "开源协议",
        runjam: "MIT",
        aionui: "Apache-2.0",
        cursor: "闭源",
      },
      {
        feature: "一键安装 Agent",
        runjam: "支持",
        aionui: "需手动配置",
        cursor: "内置",
      },
    ],
    note: `<strong>核心理念：</strong> RunJam 不与 Agent 建网络协议连接，而是直接管理 Agent 的 CLI 进程（stdin/stdout 管道）。这意味着 <strong>Agent 无需任何改造</strong>，用原生 CLI 即可接入，同时也保留了未来对接 ACP 协议的扩展空间。`,
  },
  faq: {
    title: "常见问题",
    items: [
      {
        q: "RunJam 与 AionUI / Cursor / Copilot 有何不同？",
        a: "RunJam 是本地 AI Agent 的统一管理器，类似 Docker Desktop 管理容器一样管理 Agent。与 AionUI 不同，RunJam 无需 Agent 做任何 ACP 协议改造，直接用原生 CLI 即可工作。与 Cursor/Copilot 不同，RunJam 本身不是 AI，而是让你已有的 AI Agent 更高效的平台。",
      },
      {
        q: "我的数据会发送到云端吗？",
        a: "完全不会。RunJam 是本地优先的应用，所有 Agent 进程都在你的机器上运行，所有数据（会话记录、模型配置、Agent 状态）都存储在你本地的 ~/.runjam 目录下。没有遥测，没有云端同步。",
      },
      {
        q: "支持哪些 AI Agent？",
        a: "目前支持 Claude Code、Codex CLI、Gemini CLI。RunJam 会自动检测你 PATH 中已安装的 Agent。如果你还没安装，RunJam 可以帮你一键安装。更多 Agent 支持正在规划中。",
      },
      {
        q: "RunJam 是免费的吗？",
        a: "是的，RunJam 完全开源免费，采用 MIT 协议。你可以在 GitHub 上查看所有源代码。",
      },
      {
        q: "需要什么系统环境？",
        a: "macOS、Windows、Linux 均可。唯一的前置条件是 Node.js ≥ 18，因为 Agent CLI 工具都基于 Node.js。RunJam 会帮你检查并引导安装。",
      },
      {
        q: "多个 Agent 可以同时运行吗？",
        a: "可以。你可以为不同项目创建不同的会话，每个会话使用不同的 Agent，它们独立并行运行，互不干扰。",
      },
    ],
  },
  footer: {
    title1: "一个桌面，",
    title2: "驾驭你的所有 AI Agent",
    subtitle: "开源免费 · 本地优先 · 无需改造 Agent",
    download: "下载 RunJam",
    star: "Star on GitHub",
    tagline: "RunJam — Local-first AI Agent Manager",
    license: "MIT License",
    copyright: "© 2025 RunJam",
  },
};

export default zh;
