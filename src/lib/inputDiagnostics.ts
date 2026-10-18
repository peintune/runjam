/** Local, content-free evidence for clicks that fail to become text input. */
export interface InputDiagnostic {
  stage: "after_click" | "after_wait";
  target: "input" | "textarea" | "contenteditable";
  disabled: boolean;
  readOnly: boolean;
  documentFocused: boolean;
  targetActive: boolean;
  visible: boolean;
  keyEvents: number;
  beforeInputEvents: number;
  inputEvents: number;
  elapsedMs: number;
}

export function installInputDiagnostics(report: (snapshot: InputDiagnostic) => void): () => void {
  let timers: ReturnType<typeof setTimeout>[] = [];
  let attempt: { element: HTMLElement; start: number; keys: number; before: number; inputs: number } | null = null;
  let lastClick = -Infinity;

  function clearAttempt() {
    timers.forEach(clearTimeout);
    timers = [];
    attempt = null;
  }

  function capture(stage: InputDiagnostic["stage"]) {
    if (!attempt) return;
    const { element, start, keys, before, inputs } = attempt;
    const field = element as HTMLInputElement | HTMLTextAreaElement;
    report({
      stage,
      target: element.tagName === "TEXTAREA" ? "textarea" : element.tagName === "INPUT" ? "input" : "contenteditable",
      disabled: field.disabled === true,
      readOnly: field.readOnly === true,
      documentFocused: document.hasFocus(),
      targetActive: document.activeElement === element,
      visible: document.visibilityState === "visible",
      keyEvents: keys,
      beforeInputEvents: before,
      inputEvents: inputs,
      elapsedMs: Math.round(performance.now() - start),
    });
    if (stage === "after_wait") clearAttempt();
  }

  function onPointerDown(event: Event) {
    // Never read text, keys, values, labels, paths, or placeholders.
    const element = (event.target as Element | null)?.closest?.("input, textarea, [contenteditable='true']") as HTMLElement | null;
    if (!element) { clearAttempt(); return; }
    const now = performance.now();
    if (now - lastClick < 2000) { clearAttempt(); return; }
    lastClick = now;
    clearAttempt();
    attempt = { element, start: now, keys: 0, before: 0, inputs: 0 };
    timers = [setTimeout(() => capture("after_click"), 50), setTimeout(() => capture("after_wait"), 1200)];
  }
  function onKey() { if (attempt) attempt.keys++; }
  function onBeforeInput() { if (attempt) attempt.before++; }
  function onInput() { if (attempt) attempt.inputs++; }

  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("keydown", onKey, true);
  document.addEventListener("beforeinput", onBeforeInput, true);
  document.addEventListener("input", onInput, true);
  return () => {
    clearAttempt();
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("keydown", onKey, true);
    document.removeEventListener("beforeinput", onBeforeInput, true);
    document.removeEventListener("input", onInput, true);
  };
}
