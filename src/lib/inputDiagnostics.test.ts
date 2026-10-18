import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { installInputDiagnostics } from "./inputDiagnostics";

describe("local input diagnostics", () => {
  let doc: EventTarget & { activeElement: unknown; hasFocus: () => boolean; visibilityState: string };
  let stop: (() => void) | undefined;
  const report = vi.fn();

  function pointer(element: unknown) {
    const event = new Event("pointerdown");
    Object.defineProperty(event, "target", { value: element });
    doc.dispatchEvent(event);
  }
  function field(disabled = false) {
    const element = {
      tagName: "TEXTAREA", disabled, readOnly: false,
      closest: () => element,
      get value(): never { throw new Error("input content must never be read"); },
    };
    return element;
  }

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    report.mockClear();
    doc = Object.assign(new EventTarget(), { activeElement: null, hasFocus: () => false, visibilityState: "visible" });
    vi.stubGlobal("document", doc);
    stop = installInputDiagnostics(report);
  });
  afterEach(() => { stop?.(); vi.unstubAllGlobals(); vi.useRealTimers(); });

  it("distinguishes a focus failure from a disabled input without reading content", () => {
    pointer(field());
    vi.advanceTimersByTime(1200);
    expect(report).toHaveBeenCalledTimes(2);
    expect(report.mock.calls[1][0]).toEqual({
      stage: "after_wait", target: "textarea", disabled: false, readOnly: false,
      documentFocused: false, targetActive: false, visible: true,
      keyEvents: 0, beforeInputEvents: 0, inputEvents: 0, elapsedMs: 1200,
    });
    vi.advanceTimersByTime(1000);
    pointer(field(true));
    vi.advanceTimersByTime(50);
    expect(report.mock.calls[2][0].disabled).toBe(true);
  });

  it("counts keyboard and edit events without retaining typed characters", () => {
    const element = field();
    doc.activeElement = element;
    doc.hasFocus = () => true;
    pointer(element);
    for (const type of ["keydown", "beforeinput", "input"]) doc.dispatchEvent(new Event(type));
    vi.advanceTimersByTime(1200);
    expect(report.mock.calls[1][0]).toMatchObject({
      documentFocused: true, targetActive: true, keyEvents: 1, beforeInputEvents: 1, inputEvents: 1,
    });
  });

  it("bounds repeated clicks and cancels pending reports when clicking elsewhere or disposing", () => {
    pointer(field());
    vi.advanceTimersByTime(50);
    for (let i = 0; i < 20; i++) pointer(field());
    vi.advanceTimersByTime(1500);
    expect(report).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    pointer(field());
    pointer({ closest: () => null });
    vi.advanceTimersByTime(1200);
    expect(report).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    pointer(field());
    stop?.();
    doc.dispatchEvent(new Event("keydown"));
    vi.advanceTimersByTime(1500);
    expect(report).toHaveBeenCalledTimes(1);
  });
});
