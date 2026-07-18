import { defineStore } from "pinia";
import { ref } from "vue";

export interface InteractionOption { key: string; label: string; is_default: boolean; }
export interface ToolCall { toolName: string; input: string; output?: string; status: string; }
export interface Message {
  role: "user" | "agent"; content: string; thinking?: string;
  thoughtDuration?: string;
  interaction?: { prompt: string; options: InteractionOption[]; sessionId: string; };
  toolCalls?: ToolCall[];
}

export const useMessageStore = defineStore("message", () => {
  const messagesBySession = ref<Record<string, Message[]>>({});

  function getMessages(sessionId: string): Message[] {
    return messagesBySession.value[sessionId] || [];
  }

  function setMessages(sessionId: string, messages: Message[]) {
    messagesBySession.value[sessionId] = messages;
  }

  function addMessage(sessionId: string, message: Message) {
    if (!messagesBySession.value[sessionId]) {
      messagesBySession.value[sessionId] = [];
    }
    messagesBySession.value[sessionId].push(message);
  }

  function updateLastAgentMessage(sessionId: string, updates: Partial<Message>) {
    const messages = messagesBySession.value[sessionId];
    if (!messages || messages.length === 0) return;
    
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === "agent") {
        Object.assign(messages[i], updates);
        break;
      }
    }
  }

  function clearMessages(sessionId: string) {
    messagesBySession.value[sessionId] = [];
  }

  function removeSession(sessionId: string) {
    delete messagesBySession.value[sessionId];
  }

  function getMessageTexts(sessionId: string): string[] {
    const msgs = messagesBySession.value[sessionId] || [];
    return msgs.filter(m => m.content).map(m => m.content);
  }

  function searchAll(query: string): { sessionId: string; text: string }[] {
    const q = query.toLowerCase();
    const results: { sessionId: string; text: string }[] = [];
    for (const [sid, msgs] of Object.entries(messagesBySession.value)) {
      for (const m of msgs) {
        if (m.content && m.content.toLowerCase().includes(q)) {
          results.push({ sessionId: sid, text: m.content });
        }
      }
    }
    return results;
  }

  return {
    messagesBySession,
    getMessages,
    setMessages,
    addMessage,
    updateLastAgentMessage,
    clearMessages,
    removeSession,
    getMessageTexts,
    searchAll,
  };
});
