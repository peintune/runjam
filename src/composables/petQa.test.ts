import { describe, expect, it } from "vitest";
import { petLaunchOptions, petConfigKey, defaultPetConfig, normalizePetConfig, petRecentHistory } from "./usePetChat";

describe("pet Q&A profile", () => {
  it("defaults both new and older pet settings to Q&A while preserving Agent preferences", () => {
    expect(defaultPetConfig().chatMode).toBe("qa");
    const saved = normalizePetConfig({directory: "/work", permissionMode: "full_access", skills: ["custom"], noThinking: false});
    expect(saved.chatMode).toBe("qa");
    expect(saved.directory).toBe("/work");
    expect(saved.skills).toEqual(["custom"]);
  });
  it("uses a scratch directory, no skills and session-local reasoning for Q&A", () => {
    const config = {...defaultPetConfig(), directory: "/work", skills: ["custom"], permissionMode: "full_access"};
    expect(petLaunchOptions(config)).toEqual({directory: undefined, skills: undefined, purpose: "pet_qa", reasoningDisabled: true});
    expect(petConfigKey(config)).toBe(petConfigKey({...config, skills: [], directory: "", permissionMode: "read_only", noThinking: true}));
    expect(petConfigKey(config)).not.toBe(petConfigKey({...config, chatMode: "agent"}));
  });
  it("restores the user's original Agent configuration on an explicit switch", () => {
    const config = {...defaultPetConfig(), chatMode: "agent" as const, directory: "/work", skills: ["custom"]};
    expect(petLaunchOptions(config)).toEqual({directory: "/work", skills: ["custom"], purpose: undefined, reasoningDisabled: false});
  });
  it("bounds restored Q&A history without changing Agent history", () => {
    const messages = Array.from({length: 8}, (_, i) => ({role: "user" as const, content: `${i}`.repeat(6000)}));
    const qa = petRecentHistory(messages, true);
    expect(qa.join("").length).toBeLessThanOrEqual(8000);
    expect(qa[qa.length - 1]).toContain("7".repeat(6000));
    expect(petRecentHistory(messages, false)).toHaveLength(4);
  });
  it("handles a single remaining history character without exceeding the cap", () => {
    const messages = [{role:"user" as const, content:"old".repeat(5000)}, {role:"user" as const, content:"x".repeat(7993)}];
    expect(petRecentHistory(messages, true).join("").length).toBeLessThanOrEqual(8000);
  });
});
