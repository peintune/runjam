export type StreamPresentation = "animated" | "instant-stream";

export function streamingSeconds(now: number, startedAt: number): number {
  return Math.max(0, Math.floor((now - startedAt) / 1000));
}

/** Q&A should display received text, not simulate slower generation. */
export function shouldAnimateStream(message: {content?: string; thinking?: string}, mode: StreamPresentation): boolean {
  if (mode === "instant-stream") return false;
  const content = message.content || "";
  const thinking = message.thinking || "";
  return content.length + thinking.length <= 1500 && !/```|~~~/.test(content) && !/```|~~~/.test(thinking);
}
