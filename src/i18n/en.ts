const en = {
  nav: {
    features: "Features",
    architecture: "Architecture",
    compare: "Compare",
    faq: "FAQ",
    github: "GitHub",
    language: "中文",
  },
  hero: {
    badge: "Open Source · Local First · MIT License",
    title1: "One Desktop,",
    title2: "All Your AI Agents",
    title3: "Working Together.",
    subtitle:
      "RunJam manages your AI Coding Agents just like Docker Desktop manages containers — auto-detect, one-click install, unified chat interface. Let <strong>Claude Code</strong>, <strong>Codex CLI</strong>, and <strong>Gemini CLI</strong> collaborate in one place.",
    download: "Download for macOS",
    viewSource: "View Source",
  },
  terminal: {
    prompt: '"Help me write a Rust TCP server"',
    running: "Claude Code running...",
    thinking: "Thinking: analyzing requirements, planning structure, handling edge cases...",
    tool: "Tool: write_to_file → src/server.rs",
    done: "Done · Implemented TCP server with concurrent connections and graceful shutdown",
  },
  features: {
    title: "Simpler Than You Think",
    subtitle:
      "No agent modifications. No complex configuration. This is how your AI agents should be managed.",
    cards: [
      {
        title: "Zero Config, Ready to Go",
        desc: "Auto-detects AI agents already installed on your system. No manual path or parameter configuration needed. As simple as managing containers with Docker Desktop.",
      },
      {
        title: "Multi-Agent Management",
        desc: "Claude Code, Codex CLI, Gemini CLI — all in one interface. Switching agents is as natural as switching chat partners.",
      },
      {
        title: "Real-time Streaming",
        desc: "Agent thinking process, tool calls, and final responses are all streamed in real-time, with Markdown rendering and syntax highlighting.",
      },
      {
        title: "Local First, Data Secure",
        desc: "All conversation data stored locally. Agent processes run on your machine. No telemetry collection, no cloud dependency.",
      },
      {
        title: "Unified Model Config",
        desc: "Configure once, sync across all agents. Supports OpenAI, Anthropic, Google, DeepSeek, and other major providers.",
      },
      {
        title: "Cross-Platform Desktop",
        desc: "Built with Tauri (Rust + Vue 3). Full macOS / Windows / Linux support. Small bundle size, excellent performance.",
      },
    ],
  },
  agents: {
    title: "The Best AI Agents, All in One Place",
    subtitle:
      "RunJam auto-detects and installs in one click. No manual path or environment variable setup needed.",
    steps: [
      "Scan PATH environment",
      "Auto-detect installed agents",
      "Not installed? One-click install",
      "Ready to go!",
    ],
  },
  architecture: {
    title: "Built for Performance",
    subtitle: "Tauri + Rust + Vue 3 — lightweight, fast, secure.",
    cards: [
      {
        title: "Rust Core",
        desc: "Agent process management, stdout parsing, and file system operations are all handled by Rust. Zero GC pauses, memory usage under 50MB.",
      },
      {
        title: "Vue 3 Frontend",
        desc: "Reactive UI + Tailwind CSS v4 + TypeScript. Smooth Markdown rendering, syntax highlighting, and streaming typewriter effect.",
      },
      {
        title: "Tauri Framework",
        desc: "90% smaller bundle than Electron. Native system API calls. Consistent experience across macOS, Windows, and Linux.",
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
      label: "Unified Chat Interface",
      title: "Chat with Multiple Agents<br>Just Like Messaging",
      desc: "Each message is organized by project. Agent type badges are clear at a glance. Full Markdown rendering, syntax highlighting, Mermaid diagrams, and real-time thinking process display.",
      bullets: [
        "Real-time streaming output + typewriter effect",
        "Thinking steps separated, auto-collapsed",
        "Tool call details, expand to see input/output",
        "Code block highlighting + one-click copy",
      ],
    },
    model: {
      label: "Unified Model Hub",
      title: "Configure Models Once<br>Auto-Sync to All Agents",
      desc: "Add model configurations in RunJam, automatically synced to Claude Code, Codex CLI, and Gemini CLI config files. Supports OpenAI, Anthropic, Google, Groq, DeepSeek, and custom APIs.",
      providers: ["OpenAI", "Anthropic", "Google AI", "Groq", "DeepSeek"],
      custom: "+ Custom",
    },
  },
  compare: {
    title: "Why Choose RunJam",
    subtitle:
      "No agent modifications. No ACP protocol. Works with native CLI out of the box.",
    tableHead: {
      feature: "Feature",
    },
    rows: [
      {
        feature: "Agent Modification Required",
        runjam: "None Needed",
        aionui: "ACP Protocol",
        cursor: "N/A",
      },
      {
        feature: "Multi-Agent Management",
        runjam: "Unified UI",
        aionui: "Unified UI",
        cursor: "Single Agent",
      },
      {
        feature: "Local Data Storage",
        runjam: "Fully Local",
        aionui: "Fully Local",
        cursor: "Cloud Sync",
      },
      {
        feature: "Model Config Sync",
        runjam: "One config, all synced",
        aionui: "Per-backend config",
        cursor: "Built-in models",
      },
      {
        feature: "Desktop Framework",
        runjam: "Tauri (Rust)",
        aionui: "Electron (Node)",
        cursor: "Electron",
      },
      {
        feature: "License",
        runjam: "MIT",
        aionui: "Apache-2.0",
        cursor: "Proprietary",
      },
      {
        feature: "One-Click Agent Install",
        runjam: "Yes",
        aionui: "Manual setup",
        cursor: "Built-in",
      },
    ],
    note: `<strong>Core Philosophy:</strong> RunJam does not connect to agents over network protocols — it directly manages agent CLI processes (stdin/stdout pipes). This means <strong>agents require zero modifications</strong> and work with native CLI out of the box, while keeping the door open for future ACP protocol integration.`,
  },
  faq: {
    title: "FAQ",
    items: [
      {
        q: "How is RunJam different from AionUI / Cursor / Copilot?",
        a: "RunJam is a unified local AI agent manager, similar to how Docker Desktop manages containers. Unlike AionUI, RunJam requires no ACP protocol modifications from agents — it works with native CLI directly. Unlike Cursor/Copilot, RunJam itself is not an AI, but a platform that makes your existing AI agents more efficient.",
      },
      {
        q: "Will my data be sent to the cloud?",
        a: "Not at all. RunJam is local-first. All agent processes run on your machine, and all data (session records, model configs, agent states) is stored in your local ~/.runjam directory. No telemetry, no cloud sync.",
      },
      {
        q: "Which AI agents are supported?",
        a: "Currently supports Claude Code, Codex CLI, and Gemini CLI. RunJam auto-detects agents already installed in your PATH. If not installed, RunJam can install them for you in one click. More agent support is planned.",
      },
      {
        q: "Is RunJam free?",
        a: "Yes, RunJam is fully open-source and free under the MIT license. You can view all source code on GitHub.",
      },
      {
        q: "What are the system requirements?",
        a: "macOS, Windows, and Linux are all supported. The only prerequisite is Node.js ≥ 18, since agent CLI tools are Node.js-based. RunJam will check and guide you through installation.",
      },
      {
        q: "Can multiple agents run simultaneously?",
        a: "Yes. You can create different sessions for different projects, each using a different agent. They run independently in parallel without interfering with each other.",
      },
    ],
  },
  footer: {
    title1: "One Desktop,",
    title2: "Master All Your AI Agents",
    subtitle: "Open Source & Free · Local First · Zero Agent Modifications",
    download: "Download RunJam",
    star: "Star on GitHub",
    tagline: "RunJam — Local-first AI Agent Manager",
    license: "MIT License",
    copyright: "© 2025 RunJam",
  },
};

export type Locale = typeof en;
export default en;
