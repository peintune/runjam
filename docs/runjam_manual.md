# RunJam 软件操作手册

> RunJam 是一个**本地优先、跨平台**的 AI 助手桌面管理器。它把 Claude Code、Codex CLI、Gemini CLI 等 AI Agent 统一到一个界面里管理：一套模型配置，多项目、多 Agent 并行会话。本文面向普通用户，介绍日常操作。

---

## 目录

1. [安装与启动](#1-安装与启动)
2. [界面总览](#2-界面总览)
3. [快速上手：开始第一次对话](#3-快速上手开始第一次对话)
4. [会话管理](#4-会话管理)
5. [项目工作区](#5-项目工作区)
6. [Agent 管理](#6-agent-管理)
7. [模型配置](#7-模型配置)
8. [用量统计](#8-用量统计)
9. [通用设置](#9-通用设置)
10. [快捷键速查](#10-快捷键速查)
11. [数据与隐私](#11-数据与隐私)
12. [常见问题与故障排查](#12-常见问题与故障排查)

---

## 1. 安装与启动

### 开发环境要求

- **Node.js ≥ 18**（运行 AI Agent CLI 必需）
- 系统依赖：macOS 需 Xcode Command Line Tools；Windows 需 Visual Studio C++ Build Tools + WebView2；Linux 需 `webkit2gtk` 相关包

### 安装方式

**方式 A：下载安装包（推荐）**

从 [GitHub Releases](https://github.com/peintune/runjam/releases) 页面下载对应平台安装包：
- macOS → `.dmg`
- Windows → `.msi` / `.exe`
- Linux → `.deb` / `.AppImage`

**方式 B：从源码编译（开发者）**

```bash
git clone https://github.com/peintune/runjam.git
cd runjam
npm install
npm run tauri dev        # 开发模式（热更新）
npm run tauri build      # 编译安装包，产物在 src-tauri/target/release/bundle/
```

### 首次启动

1. 打开 RunJam，应用会自动检测系统中已安装的 AI Agent（Claude Code、Codex CLI、Gemini CLI）
2. 进入 **设置 → Agents**，一键安装缺失的 Agent
3. 进入 **设置 → Models**，添加 API 密钥和模型
4. 点击 **New Session** 开始对话

---

## 2. 界面总览

```
┌─────────────────────────────────────────────────────────────┐
│ 顶栏：侧边栏开关 | 🔍搜索 | 📁文件树 | 终端 | 窗口控制       │
├──────────────┬──────────────────────────────────────────────┤
│ 侧边栏       │  会话区（聊天 / 新会话落地页）                │
│  · New Session│                                             │
│  · Directory │   ┌───────── 工作区面板（可选）────────────┐  │
│  · Conversations│ │ 文件树 │ 编辑器 │ 终端                 │  │
│  · Archived  │   └───────────────────────────────────────┘  │
│  · Costs     │                                              │
│  · Settings  │                                              │
└──────────────┴──────────────────────────────────────────────┘
```

- **顶栏**：左侧折叠/展开侧边栏；中间是搜索、文件树、终端开关；右侧窗口控制按钮
- **侧边栏**：
  - 顶部：Logo、批量选择按钮、紧凑/舒适视图切换
  - 中部：**New Session** 大按钮（回到新建会话页）；会话树分为 **Directory**（按项目分组）、**Conversations**（无目录会话）、**Archived**（归档区）三组
  - 底部：**Costs**（用量统计）和 **Settings**（设置）入口
- **会话区**：没有活动会话时显示新建会话页；有会话时显示消息流和输入栏

---

## 3. 快速上手：开始第一次对话

### 新建会话

点击侧边栏的 **New Session**，进入新建会话页：

1. **选择 Agent**：顶部选择条中点击要使用的 Agent 药丸按钮（如 Claude、Codex、Gemini）
2. **选择项目文件夹（可选）**：点击输入框下方的 "work in a project"，选择最近项目、无项目或 "Open a new folder..."；选中的目录会显示在输入框旁，可一键清除
3. **选择模型**：输入框上方的模型下拉中挑选模型（未选模型时发送按钮禁用并提示 "Please select a model"）
4. **可选设置**：权限模式（Plan Mode / Accept Edits / Auto Mode / Bypass Permissions 等，随 Agent 不同）、思考模式开关
5. **输入消息**：在输入框中输入内容，按 **Enter** 发送（**Shift+Enter** 换行）

> 发送后会**自动创建会话**，标题取输入内容的前 30 字。

### 输入小技巧

- **@ 引用文件**：在输入框行首或空白后输入 `@`，弹出文件选择器，支持 File / Folders 两个标签页，可继续打字搜索；`↑↓` 选择、`Enter`/`Tab` 确认、`Esc` 关闭。选中后输入框会插入 `@相对路径`，发送时自动展开为绝对路径
- **技能（Skills）**：输入框上方有技能标签行，点魔杖按钮可弹出技能多选卡片，为会话附加技能

---

## 4. 会话管理

### 常用操作

将鼠标悬停在会话上，点击出现的 `...` 菜单按钮（或右键），可执行：

- **Pin to top / Unpin**：置顶 / 取消置顶
- **Rename**：重命名会话（也可直接点击会话标题改名）
- **Archive / Unarchive**：归档 / 恢复归档
- **Delete**：删除会话（**无二次确认，请谨慎操作**）

### 批量操作

点击侧边栏顶部的批量选择按钮（CheckSquare 图标），可勾选多个会话，批量 **置顶 / 归档 / 删除**。

### 归档区

- 归档会话显示在侧边栏 **Archived** 分组，可随时恢复
- "删除全部归档"按钮需要**两次点击确认**（3 秒内）才会执行

### 搜索会话

点击顶栏的 **🔍 放大镜图标**（搜索没有快捷键），弹出居中搜索浮层，实时搜索所有会话的消息内容（200ms 防抖）；点击结果直接跳转到对应会话。清空搜索词时显示最近 12 个会话。

> 注意：一个会话的 Agent 在创建后是**固定的**，不能中途切换。想用别的 Agent，回到 New Session 另开会话即可。

---

## 5. 项目工作区

> 只有**绑定了项目目录**的会话才能启用工作区。

- **文件树**：点击顶栏 📁 图标打开/关闭。头部显示项目路径，支持 "Open in Finder"、刷新和文件搜索
- **编辑器**：点击文件树中的文件打开（Monaco 编辑器，支持语法高亮）。多文件以标签页展示，点击切换、中键或 × 关闭；**Cmd/Ctrl+S 保存**。图片/PDF 使用预览模式，Office 文件直接在系统默认程序中打开
- **终端**：点击顶栏终端图标打开（xterm.js），支持多标签（+ 新建、× 关闭、清屏）。每个标签按目录持久化；关闭终端时会提示将终止所有终端进程
- 侧边栏宽度、文件树宽度、终端高度均可**拖拽调整**

---

## 6. Agent 管理

入口：侧边栏底部 **Settings → Agents**。

### 安装 / 卸载

- 未安装的 Agent 显示 **Install** 按钮，点击一键安装（`npm install -g`），实时显示进度和日志
- 已安装的 Agent 显示 **Test**（测试连接）和 **Uninstall**（卸载）

### 启用 / 禁用

每张 Agent 卡片有 **Enabled / Disabled** 开关：禁用的 Agent 不会出现在新建会话的选择条中，但已创建的会话不受影响。

### Agent 详情页

点击 Agent 卡片进入详情页，可查看：

- **基本信息**：状态徽章、版本、安装路径、官方网址和安装命令
- **Model Configuration**：从全部模型中挑选一个 "Apply" 分配给该 Agent；已分配的模型可移除
- **Configuration File**：直接编辑该 Agent 的 JSON 配置文件（如 Claude Code 的 `~/.claude/settings.json`），点 **Save Changes** 保存
- **Operation Log**：查看安装/操作日志

---

## 7. 模型配置

入口：侧边栏底部 **Settings → Models**（默认设置页）。

### 添加云服务模型

点击 **Add Model**，选择服务商预设（OpenAI、Anthropic、Google AI、Groq、DeepSeek、自定义 API 等），填入：

- **API Key**：掩码显示，眼睛图标可切换可见；密钥安全存储在操作系统钥匙串中
- **API Base**：自定义 API 服务地址
- **模型 ID / 名称**

每个模型卡片显示协议标签（Chat / Responses / Anthropic / Gemini 等），以及 **Agents: N** 分配下拉——勾选要使用该模型的 Agent。

> 模型**配置一次，自动同步到所有 Agent**。RunJam 内置本地代理自动转换 LLM 协议（Anthropic ↔ OpenAI ↔ Gemini），让任何 Agent 都能使用任何模型。

### 本地模型（llama.cpp）

100% 本地免费运行开源模型（DeepSeek Coder、Qwen、Llama 等）：

1. 若 `llama-server` 二进制缺失，设置页顶部会出现黄色警告条并提供下载链接
2. 在 **Local Models** 区块选择推荐模型（如 Ornith-1.0-9B-GGUF），点击 **Download** 下载（带进度条）
3. 下载完成后点击 **Start** 启动本地服务器（启动时有全屏加载浮层和实时日志）
4. 运行中的模型会自动加入模型列表；在会话的模型下拉中，本地模型单独显示在 **Local Models** 分组（带绿色运行状态圆点）。未运行时显示 "Start server"，点击可跳转到模型设置页启动

- "Add Model" 可手动添加 GGUF 模型（从 Hugging Face 下载后放入模型文件夹）
- "Open Folder" 打开模型目录；已下载的模型可 **Start / Stop** 随时启停

---

## 8. 用量统计

入口：侧边栏底部 **Costs**。

- **摘要卡片**：Today / This Week / This Month / All Time 的 token 用量
- **Cache Performance**：今日/本周/全部 KV 缓存命中率（进度条显示），帮助了解缓存优化效果
- **图表**：By Agent 环形图、Daily Trend 7/14/30/90 天柱状图
- **明细表格**：By Day / By Agent / By Session / By Project 四个标签页，包含 Tokens、Cache Hit 率、Sessions、Msgs 等列
- 右上角 **Refresh** 刷新数据

---

## 9. 通用设置

入口：侧边栏底部 **Settings → General**。

- **Appearance**：外观（当前仅 Light 模式）
- **Data Directory**：显示数据目录 `~/.runjam` 路径，点击 Open 打开
- **Anonymous Usage Data**：匿名用量数据开关（默认关闭，本地优先，无遥测）
- **Version**：版本号

---

## 10. 快捷键速查

| 快捷键 | 功能 |
|--------|------|
| `Ctrl/Cmd + Shift + D` | 切换性能诊断浮层（显示事件速率、handler 耗时、主线程阻塞等） |
| `Cmd/Ctrl + S` | 保存当前编辑器文件（仅在编辑器打开时生效） |
| `Enter` | 发送消息（输入框内） |
| `Shift + Enter` | 输入框换行 |
| `@` | 在输入框弹出文件引用选择器 |

搜索功能无快捷键，需点击顶栏放大镜图标。

---

## 11. 数据与隐私

- **本地优先**：所有对话、配置和 Agent 状态都存储在 `~/.runjam/`
- **零遥测**：无数据收集、无分析、无回传
- **无云依赖**：完全可离线工作（Agent 调用 API 时需要各自的网络访问）
- **API 密钥安全**：存储在操作系统钥匙串中，不落明文
- **免费开源**：MIT 许可证

---

## 12. 常见问题与故障排查

### 提示 "Please select a model" 无法发送？

还没有为当前 Agent 分配模型。到 **设置 → Models** 添加模型并分配给该 Agent，或在模型下拉中直接选择 / 直达 "Add Model"。

### Agent 没有出现在选择条里？

到 **设置 → Agents** 检查是否已安装、是否被禁用。

### macOS 提示 "RunJam 已损坏，无法打开"？

macOS Gatekeeper 拦截了未签名应用。在终端运行：

```bash
xattr -cr /Applications/RunJam.app
```

### 界面卡顿、消息渲染慢？

按 `Ctrl/Cmd + Shift + D` 打开诊断浮层，查看事件处理耗时、长任务（≥50ms 主线程阻塞）等指标，帮助定位问题。再次按快捷键关闭。

### 我的数据会被上传到云端吗？

**不会。** 所有数据都在本地 `~/.runjam/`，无遥测、无云同步。

### 系统要求？

macOS、Windows、Linux 均支持，唯一前提是 **Node.js ≥ 18**（AI Agent CLI 需要）。RunJam 会检测并在需要时引导安装。

---

*本手册基于 RunJam 当前版本编写。更多信息见 [README.zh-CN.md](README.zh-CN.md) 和 [GitHub 仓库](https://github.com/nicepkg/runjam)。*
