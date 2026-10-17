import { beforeEach, describe, expect, it, vi } from "vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import { openExternalContentLink } from "./contentLinks";

// Only the OS browser launch is mocked; navigation cancellation stays real.
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

function linkEvent(href: string | null, options: { button?: number; textNode?: boolean; type?: string; metaKey?: boolean } = {}) {
  const anchor = href === null ? null : { getAttribute: () => href };
  const element = { nodeType: 1, closest: (selector: string) => selector === "a[href]" ? anchor : null };
  const event = new Event(options.type ?? "click", { cancelable: true });
  Object.defineProperties(event, {
    target: { value: options.textNode ? { nodeType: 3, parentElement: element } : element },
    button: { value: options.button ?? 0 },
    metaKey: { value: options.metaKey ?? false },
  });
  return event as MouseEvent;
}

beforeEach(() => {
  vi.mocked(openUrl).mockReset().mockResolvedValue(undefined);
});

describe("message links in the shared main/pet transcript", () => {
  it.each(["https://example.com/docs?q=hello#part", "http://example.com/"])("opens %s externally and cancels webview navigation synchronously", (url) => {
    const event = linkEvent(url);
    expect(openExternalContentLink(event)).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(vi.mocked(openUrl).mock.calls).toEqual([[url]]);
  });

  it("handles a nested text target and Cmd-click without opening an in-app tab", () => {
    const event = linkEvent("https://example.com/", { textNode: true, metaKey: true });
    openExternalContentLink(event);
    expect(event.defaultPrevented).toBe(true);
    expect(vi.mocked(openUrl).mock.calls).toEqual([["https://example.com/"]]);
  });

  it("opens a middle-click externally, but does not launch a browser for right-click", () => {
    const middle = linkEvent("https://example.com/", { button: 1, type: "auxclick" });
    openExternalContentLink(middle);
    expect(middle.defaultPrevented).toBe(true);
    expect(vi.mocked(openUrl).mock.calls).toEqual([["https://example.com/"]]);
    const right = linkEvent("https://example.com/", { button: 2, type: "auxclick" });
    expect(openExternalContentLink(right)).toBe(false);
    expect(openUrl).toHaveBeenCalledTimes(1);
  });

  it("normalizes a protocol-relative website to HTTPS", () => {
    const event = linkEvent("//example.com/docs");
    openExternalContentLink(event);
    expect(event.defaultPrevented).toBe(true);
    expect(vi.mocked(openUrl).mock.calls).toEqual([["https://example.com/docs"]]);
  });

  it.each(["mailto:hello@example.com", "tel:+123456789"])("preserves supported system-app links: %s", (url) => {
    const event = linkEvent(url);
    openExternalContentLink(event);
    expect(event.defaultPrevented).toBe(true);
    expect(vi.mocked(openUrl).mock.calls).toEqual([[url]]);
  });

  it.each(["javascript:alert(1)", "data:text/html,test", "file:///tmp/test", "/settings", "docs/page", "https://", ""])("blocks unsafe or app-relative navigation: %s", (url) => {
    const event = linkEvent(url);
    expect(openExternalContentLink(event)).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(openUrl).not.toHaveBeenCalled();
  });

  it.each([null, "#heading"])("does not intercept non-links, file-path controls or local fragments: %s", (url) => {
    const event = linkEvent(url);
    expect(openExternalContentLink(event)).toBe(false);
    expect(event.defaultPrevented).toBe(false);
    expect(openUrl).not.toHaveBeenCalled();
  });

  it("keeps the app in place when the external browser fails to open", async () => {
    const error = new Error("OS opener failed");
    vi.mocked(openUrl).mockRejectedValueOnce(error);
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      const event = linkEvent("https://example.com/");
      openExternalContentLink(event);
      expect(event.defaultPrevented).toBe(true);
      await Promise.resolve();
      expect(log).toHaveBeenCalledWith("Failed to open external content link:", error);
      expect(openUrl).toHaveBeenCalledTimes(1);
    } finally {
      log.mockRestore();
    }
  });
});
