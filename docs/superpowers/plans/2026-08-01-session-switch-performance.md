# Session Switch Performance Fix — Lazy Rendering

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the multi-second freeze when clicking a session with a very large message history.

**Architecture:** The freeze is the synchronous markdown render (`marked` + `hljs` + `DOMPurify`, cache-miss on first open) plus full DOM creation for every historical message in one tick. Fix = render only message groups near the viewport; everything else gets a cheap fixed-height placeholder. An `IntersectionObserver` (root = viewport, `rootMargin` 400px so rendering happens before the user actually scrolls there) flips a group from placeholder → fully rendered. Active/live streaming groups always render. The existing module-level markdown cache stays — revisits are instant. No windowing library, no Rust changes, no new dependencies.

**Tech Stack:** Vue 3 (script setup), TypeScript, native `IntersectionObserver`.

## Global Constraints

- No new runtime dependencies.
- Do NOT touch the Rust backend or IPC — transfer is not the bottleneck; synchronous render is.
- The module-level markdown cache in `useMarkdown.ts` must remain untouched (revisits rely on it).
- Message index `oi` (original index into `props.messages`) stays the identity used by `thinkingExpanded`/`toolExpanded`/`displayMap`/`mermaidRenderedMessages` — lazy rendering must not change indices.
- Streaming hot path unchanged: groups with any `isProcessing === true` or active typewriter message MUST always render fully (never placeholder).
- Keep changes inside `src/components/ChatMessages.vue` only.
- Existing group index `gIdx` (used by `data-user-msg-index` for `scrollToMessage`) is the identity for visibility — same indexing as the template's `v-for="(group, gIdx) in messageGroups"`.

---

### Task 1: Lazy render of off-viewport message groups

**Files:**
- Modify: `src/components/ChatMessages.vue` (script setup + template)

**Interfaces:**
- Consumes: existing `props.messages`, `messageGroups` computed, `isGroupActive()`, `renderContent()`, `displayMap`.
- Produces: `const visibleGroups = ref<Set<number>>(new Set())` (group indices currently rendered), `function ensureGroup(el: HTMLElement, gIdx: number)` (observer registration), `function resetVisibility()` (on message array swap).

**Design:**

1. **Visibility set + observer** — add to script setup:
```ts
const visibleGroups = ref<Set<number>>(new Set());
let visibilityObserver: IntersectionObserver | null = null;
const GHOST_HEIGHT = 140; // px estimate for placeholder groups

function ensureGroup(el: HTMLElement, gIdx: number) {
  // Skip observer entirely for small histories — render everything (no overhead)
  if (messageGroups.value.length <= 50) return;
  if (visibleGroups.value.has(gIdx)) return;
  if (!visibilityObserver) {
    visibilityObserver = new IntersectionObserver(
      (entries) => {
        for (const en of entries) {
          const idx = Number((en.target as HTMLElement).dataset.gIdx);
          if (en.isIntersecting) {
            visibleGroups.value.add(idx);
            visibilityObserver!.unobserve(en.target); // render once, keep rendered
          }
        }
      },
      { rootMargin: "400px 0px 400px 0px" }, // render 400px before entering viewport
    );
  }
  visibilityObserver.observe(el);
}

// A group must render fully when it's visible OR live (streaming/processing/typing)
function shouldRenderGroup(g: { items: { oi: number; msg: Message }[] }, gIdx: number): boolean {
  if (messageGroups.value.length <= 50) return true; // small session: render all
  if (visibleGroups.value.has(gIdx)) return true;
  return isGroupActive(g.items); // never placeholder a live group
}
```

2. **Reset on message array swap** — when switching sessions, `props.messages` gets a new array reference. Disconnect the old observer, clear the set, and let the new render re-observe:
```ts
watch(
  () => props.messages,
  () => {
    if (visibilityObserver) { visibilityObserver.disconnect(); visibilityObserver = null; }
    visibleGroups.value = new Set();
  },
);
```
(This watch is separate from the existing deep watchers — it fires only on reference change.)

3. **Template** — wrap each group's inner content:
```html
<template v-for="(group, gIdx) in messageGroups" :key="gIdx">
  <template v-if="shouldRenderGroup(group, gIdx)">
    <!-- existing full group markup unchanged: user bubble / agent bubble -->
  </template>
  <!-- Placeholder: cheap, fixed height, keeps scrollbar roughly stable -->
  <div
    v-else
    :data-gIdx="gIdx"
    :ref="(el) => { if (el) ensureGroup(el, gIdx as number); }"
    class="msg-row msg-row-ghost"
    :style="{ height: GHOST_HEIGHT + 'px' }"
  >
    <span class="text-gray-300 text-[13px]">…</span>
  </div>
</template>
```
Note: the placeholder div itself carries `:data-gIdx` + `:ref` so the observer watches the placeholder; once it enters the 400px buffer zone it flips to full render and stops being observed.

4. **`now` tick stays** — with lazy rendering, the 500ms `now` re-render only re-evaluates the (small) visible set; placeholders and off-screen groups do not run `renderContent`. No change needed.

5. **Mermaid post-processing** — already guarded by `mermaidRenderedMessages` set and `hasMermaid` check; lazy rendering means mermaid only hydrates for groups that actually rendered. No change needed.

**Testing** (manual — no test framework exists in this repo):
- [ ] Open a session with 200+ messages (ideally with code blocks): click must show content in < 200ms, no freeze.
- [ ] Scroll up through history: placeholders flip to rendered content smoothly within ~400px before they enter view; no blank gaps at the scroll point.
- [ ] Scroll while a session is streaming (live group at bottom): live group always renders, no placeholder flash.
- [ ] Switch to another session, switch back: instant render (markdown cache hit) — observer re-registers without error.
- [ ] Small session (< 50 groups): renders everything as before, no placeholders, no behavioral change.
- [ ] `npm run build` (vue-tsc + vite) passes.

**Steps:**

- [ ] **Step 1: Add visibility state, observer, and `shouldRenderGroup` to script setup** (code above)
- [ ] **Step 2: Add the reset-on-swap watcher** (code above)
- [ ] **Step 3: Wrap the group template with `shouldRenderGroup` + placeholder branch** (code above)
- [ ] **Step 4: Run `npm run build`** — expect vue-tsc + vite to pass
- [ ] **Step 5: Manual tests** (list above)
- [ ] **Step 6: Commit**
```bash
git add src/components/ChatMessages.vue
git commit -m "perf: lazily render off-viewport message groups on session open"
```

---

## Self-Review

**Spec coverage:** "Click a large session freezes for seconds" → Task 1 renders only the visible window immediately (first paint fast) and hydrates the rest lazily via IntersectionObserver as the user scrolls. The synchronous full-render is eliminated because `renderContent()` is only invoked inside the fully-rendered branch.

**Placeholder scan:** No TBDs; every step has concrete code or a concrete manual check.

**Type consistency:** `visibleGroups: ref<Set<number>>`, `ensureGroup(el, gIdx)`, `shouldRenderGroup(group, gIdx)`, `GHOST_HEIGHT` used consistently in script and template. `gIdx` matches the existing `v-for` index and `data-user-msg-index`.
