import { invoke } from "@tauri-apps/api/core";

export type ProtocolType = "anthropic" | "openai_chat" | "openai_responses" | "gemini";

export interface OllamaModel {
  name: string;
  size: string;
  digest: string;
  modified_at: string;
  size_bytes: number;
}

export interface OllamaPullProgress {
  status: string;
  digest?: string;
  total?: number;
  completed?: number;
  percentage: number;
}

export async function checkOllamaInstalled(): Promise<boolean> {
  return invoke<boolean>("check_ollama_installed");
}

export async function getOllamaStatus(): Promise<string> {
  return invoke<string>("get_ollama_status");
}

export async function listOllamaModels(): Promise<OllamaModel[]> {
  return invoke<OllamaModel[]>("list_ollama_models");
}

export async function pullOllamaModel(modelName: string): Promise<void> {
  return invoke("pull_ollama_model", { modelName });
}

export async function createOllamaModel(modelName: string): Promise<string> {
  return invoke<string>("create_ollama_model", { modelName });
}

export const recommendedLocalModels = [
  { name: "llama3", alias: "LLaMA 3", size: "4.7GB", desc: "综合性能最强的开源模型" },
  { name: "mistral", alias: "Mistral", size: "4.1GB", desc: "代码能力优秀，响应速度快" },
  { name: "qwen2:7b", alias: "Qwen 2 7B", size: "3.8GB", desc: "中文支持优秀" },
  { name: "phi3", alias: "Phi-3", size: "2.1GB", desc: "轻量高效，适合资源有限环境" },
  { name: "codellama", alias: "CodeLlama", size: "4.7GB", desc: "专为代码任务优化" },
  { name: "gemma2", alias: "Gemma 2", size: "2.5GB", desc: "Google 开源模型，质量优秀" },
];

export interface ModelEntry {
  id: string;
  name: string;
  alias: string;
  provider: string;
  provider_name: string;
  provider_icon: string;
  api_base: string;
  api_key: string;
  protocol: ProtocolType;
}

export async function getModels(): Promise<ModelEntry[]> {
  return invoke<ModelEntry[]>("get_models");
}

export async function saveModels(models: ModelEntry[]): Promise<void> {
  return invoke("save_models", { models });
}

export async function getLastAgent(): Promise<string> {
  return invoke<string>("get_last_agent");
}

export async function setLastAgent(agentId: string): Promise<void> {
  return invoke("set_last_agent", { agentId });
}

export async function getAgentModels(agentId: string): Promise<ModelEntry[]> {
  return invoke<ModelEntry[]>("get_agent_models", { agentId });
}

export interface AgentModelInfo {
  agent_id: string;
  model_id: string;
  use_proxy: boolean;
  is_default: boolean;
}

export async function getAgentModelMap(): Promise<AgentModelInfo[]> {
  return invoke<AgentModelInfo[]>("get_agent_model_map");
}

export async function setAgentDefaultModel(agentId: string, modelId: string): Promise<void> {
  return invoke("set_agent_default_model", { agentId, modelId });
}

export async function assignModelToAgent(agentId: string, modelId: string, useProxy: boolean = true): Promise<void> {
  return invoke("assign_model_to_agent", { agentId, modelId, useProxy });
}

export async function setAgentModel(agentId: string, modelId: string): Promise<void> {
  return invoke("set_agent_model_cmd", { agentId, modelId });
}

export async function removeModelFromAgent(agentId: string, modelId: string): Promise<void> {
  return invoke("remove_model_from_agent", { agentId, modelId });
}

export async function readAgentConfigModels(agentId: string): Promise<ModelEntry[]> {
  return invoke<ModelEntry[]>("read_agent_config_models", { agentId });
}

export interface ModelAlias {
  alias: string;
  model_id: string;
  description: string;
}

export async function getModelAliases(): Promise<ModelAlias[]> {
  return invoke<ModelAlias[]>("get_model_aliases");
}

export async function addModelAlias(alias: string, modelId: string, description: string): Promise<void> {
  return invoke("add_model_alias", { alias, modelId, description });
}

export async function removeModelAlias(alias: string): Promise<void> {
  return invoke("remove_model_alias", { alias });
}

export async function getModelByAlias(alias: string): Promise<ModelEntry | null> {
  return invoke<ModelEntry | null>("get_model_by_alias", { alias });
}

export async function syncModelToAllAgents(modelId: string): Promise<void> {
  return invoke("sync_model_to_all_agents", { modelId });
}

export async function setDefaultModel(agentId: string, modelId: string): Promise<void> {
  return invoke("set_default_model", { agentId, modelId });
}

export async function getDefaultModel(agentId: string): Promise<string> {
  return invoke<string>("get_default_model", { agentId });
}

export interface ProviderConfig {
  id: string;
  name: string;
  icon: string;
  color: string;
  defaultBase: string;
  protocol: ProtocolType;
  models: string[];
  desc: string;
  homepage: string;
}

export const providers: ProviderConfig[] = [
  {
    id: "openai",
    name: "OpenAI",
    icon: "⚡",
    color: "#10a37f",
    defaultBase: "https://api.openai.com/v1",
    protocol: "openai_chat",
    models: [],
    desc: "GPT-4o, o1 series",
    homepage: "https://platform.openai.com/",
  },
  {
    id: "anthropic",
    name: "Anthropic",
    icon: "💬",
    color: "#00d4ff",
    defaultBase: "https://api.anthropic.com/v1",
    protocol: "anthropic",
    models: [],
    desc: "Claude 3.5 Sonnet, Opus",
    homepage: "https://console.anthropic.com/",
  },
  {
    id: "gemini",
    name: "Gemini",
    icon: "🌐",
    color: "#4285f4",
    defaultBase: "https://generativelanguage.googleapis.com/v1",
    protocol: "gemini",
    models: [],
    desc: "Gemini Flash, Pro",
    homepage: "https://aistudio.google.com/",
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    icon: "🔍",
    color: "#6366f1",
    defaultBase: "https://api.deepseek.com/v1",
    protocol: "openai_chat",
    models: [],
    desc: "DeepSeek V3, Coder",
    homepage: "https://platform.deepseek.com/",
  },
  {
    id: "zhipu",
    name: "Zhipu",
    icon: "🔮",
    color: "#6b7280",
    defaultBase: "https://open.bigmodel.cn/api/paas/v4",
    protocol: "openai_chat",
    models: [],
    desc: "GLM series",
    homepage: "https://open.bigmodel.cn/",
  },
  {
    id: "dashscope",
    name: "Dashscope",
    icon: "☁️",
    color: "#ff6a00",
    defaultBase: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Qwen models",
    homepage: "https://dashscope.aliyun.com/",
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    icon: "🔀",
    color: "#8b5cf6",
    defaultBase: "https://openrouter.ai/api/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Model marketplace",
    homepage: "https://openrouter.ai/",
  },
  {
    id: "custom",
    name: "Custom",
    icon: "⚙️",
    color: "#9ca3af",
    defaultBase: "",
    protocol: "openai_chat",
    models: [],
    desc: "Any OpenAI-compatible API",
    homepage: "",
  },
  {
    id: "alibaba",
    name: "Alibaba",
    icon: "☁️",
    color: "#ff6a00",
    defaultBase: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Qwen models",
    homepage: "https://dashscope.aliyun.com/",
  },
  {
    id: "tencent",
    name: "Tencent",
    icon: "🐧",
    color: "#0099cc",
    defaultBase: "https://api.tencentcloud.com/",
    protocol: "openai_chat",
    models: [],
    desc: "Tencent models",
    homepage: "https://cloud.tencent.com/",
  },
  {
    id: "moonshot",
    name: "Moonshot",
    icon: "🌙",
    color: "#3b82f6",
    defaultBase: "https://api.moonshot.cn/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Kimi models",
    homepage: "https://platform.moonshot.cn/",
  },
  {
    id: "siliconflow",
    name: "SiliconFlow",
    icon: "💎",
    color: "#a855f7",
    defaultBase: "https://api.siliconflow.cn/v1",
    protocol: "openai_chat",
    models: [],
    desc: "SiliconFlow models",
    homepage: "https://siliconflow.cn/",
  },
  {
    id: "xai",
    name: "XAI",
    icon: "🤖",
    color: "#000000",
    defaultBase: "https://api.x.ai/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Grok models",
    homepage: "https://x.ai/",
  },
  {
    id: "novita",
    name: "Novita",
    icon: "✨",
    color: "#ec4899",
    defaultBase: "https://api.novita.ai/v3",
    protocol: "openai_chat",
    models: [],
    desc: "Novita AI models",
    homepage: "https://novita.ai/",
  },
  {
    id: "groq",
    name: "Groq",
    icon: "⚡",
    color: "#10a37f",
    defaultBase: "https://api.groq.com/openai/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Groq inference",
    homepage: "https://groq.com/",
  },
  {
    id: "ollama",
    name: "Ollama",
    icon: "🦙",
    color: "#000000",
    defaultBase: "http://localhost:11434/v1",
    protocol: "openai_chat",
    models: [],
    desc: "Local models",
    homepage: "https://ollama.com/",
  },
];

export function getProviderById(id: string): ProviderConfig | undefined {
  return providers.find((p) => p.id === id);
}

export function getProviderByName(name: string): ProviderConfig | undefined {
  return providers.find((p) => p.name.toLowerCase() === name.toLowerCase());
}

export function getProviderByBaseUrl(baseUrl: string): ProviderConfig | undefined {
  return providers.find((p) => {
    if (!p.defaultBase) return false;
    try {
      return baseUrl.includes(new URL(p.defaultBase).hostname);
    } catch {
      return false;
    }
  });
}

export function getProviderLogo(providerName: string): string {
  const provider = getProviderByName(providerName);
  if (!provider) return "⚡";
  const logos: Record<string, string> = import.meta.glob('/src/assets/provider-logos/*.svg', { eager: true, import: 'default' });
  const logoPath = `/src/assets/provider-logos/${provider.id}.svg`;
  return (logos[logoPath] as string) || provider.icon || "⚡";
}

export function maskApiKey(apiKey: string): string {
  if (!apiKey) return "";
  if (apiKey.length <= 8) return apiKey;
  const prefix = apiKey.slice(0, 4);
  const suffix = apiKey.slice(-4);
  return `${prefix}****${suffix}`;
}