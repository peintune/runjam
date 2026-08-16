---
name: runjam-defaults
description: Default constraints for every RunJam session. Defines output path conventions, dependency checking rules, fallback strategies, and file management discipline. This skill is auto-injected into every session — do not remove.
---

# RunJam Default Constraints

These are **non-negotiable** defaults that apply to every session. They prevent common Agent mistakes: outputting files to wrong directories, using unavailable dependencies without a fallback, and making assumptions about the environment.

Treat every rule below as a **HARD RULE** — violating any one means the task is not done, regardless of content quality.

---

## 0. Path Convention (READ FIRST — prevents the #1 Agent mistake)

**Every relative path in this document and in every skill's SKILL.md is relative to the SESSION WORKING DIRECTORY — NOT the skill's own directory.**

| Term | Meaning |
|---|---|
| **Session working directory** | The directory where the Agent process runs. This is the session root — e.g. `~/.runjam/session/<id>/` or the user-selected project folder. All `./` paths resolve here. |
| **Skill directory** | Where the SKILL.md lives — e.g. `.claude/skills/ppt-generation/`. **NEVER create `outputs/` or `workspace/` inside a skill directory.** The skill directory is read-only resources. |

### The rule

When a SKILL.md says `./outputs/` or `./workspace/`, it means:

```
<session working directory>/outputs/      ← deliverables go HERE
<session working directory>/workspace/    ← helper scripts go HERE
```

NOT:

```
<session working directory>/.claude/skills/<skill-name>/outputs/     ← WRONG
<session working directory>/.claude/skills/<skill-name>/workspace/   ← WRONG
```

### Path safety check (MANDATORY in every generated script)

At the top of any Python script you generate, include this guard to ensure outputs land in the session root, not inside a skill folder:

```python
import os
# Ensure we're writing to the session working directory, not a skill folder.
# If the cwd contains '.claude/skills' or '.codex/skills' or '.gemini/skills',
# walk up to the session root (the parent of the .claude/.codex/.gemini dir).
_cwd = os.getcwd()
for _marker in ('.claude', '.codex', '.gemini'):
    _idx = _cwd.find(os.sep + _marker + os.sep + 'skills')
    if _idx != -1:
        os.chdir(_cwd[:_idx])  # session root = parent of .claude/.codex/.gemini
        break
SESSION_ROOT = os.getcwd()
OUTPUTS_DIR  = os.path.join(SESSION_ROOT, 'outputs')
WORKSPACE_DIR = os.path.join(SESSION_ROOT, 'workspace')
os.makedirs(OUTPUTS_DIR, exist_ok=True)
os.makedirs(WORKSPACE_DIR, exist_ok=True)
```

Then use `OUTPUTS_DIR` (an absolute path) for all deliverable paths:

```python
OUTPUT_PATH = os.path.join(OUTPUTS_DIR, "report.pptx")  # ✅ absolute, session-root
# NOT: "./outputs/report.pptx"  ← ambiguous, may land in skill dir
```

### Quick self-test before running a script

```bash
pwd                                    # should show session root, NOT .../skills/<name>
# If pwd contains 'skills/', you are in the wrong directory. cd to session root first.
mkdir -p ./outputs ./workspace         # creates in session root
```

---

## 1. Output Path Convention

**All generated files (documents, images, presentations, archives, scripts written for the user's deliverable) MUST be placed in the session working directory's `outputs/` folder — NOT inside any skill directory.**

Before creating any output file:
```bash
mkdir -p ./outputs   # run from session root (see §0)
```

Acceptable (all relative to session working directory):
- `./outputs/report.pptx`
- `./outputs/chart.png`
- `./outputs/data.xlsx`
- `./outputs/exported-doc.pdf`

Not acceptable:
- `./report.pptx` (in session root directly)
- `~/Desktop/report.pptx` (user home)
- `./tmp/result.pptx` (random subdir)
- `.claude/skills/<skill-name>/outputs/report.pptx` (**inside skill directory — FORBIDDEN**)
- Any arbitrary path outside `./outputs/` unless it's part of the project's own code structure.

**Exception:** Code files that belong to the project (e.g. under `./src/`, `./components/`, `./tests/`) follow the project's existing conventions. Config files at project root (e.g. `tsconfig.json`, `vite.config.ts`) are also fine.

**Verification step:** After producing a deliverable file, run from the session root:
```bash
ls -la ./outputs/<filename>
```
and confirm the file exists and has non-zero size. If the file is not there, check whether it accidentally landed inside `.claude/skills/*/outputs/` — if so, move it to the session root's `./outputs/` and fix your script.

---

## 2. Dependency Check Before Action

**Before using any skill, library, or CLI tool, check that its dependencies are actually available.**

### Quick dependency probe recipes

| Dependency | Probe command |
|---|---|
| `python-pptx` | `python -c "import pptx; print(pptx.__version__ if hasattr(pptx,'__version__') else 'ok')" 2>&1` |
| `officecli` | `officecli --version` |
| Any Python package | `python -c "import <pkg>" 2>&1` |
| Any binary tool | `command -v <tool>` or `which <tool>` |
| A skill's bundled scripts | `ls <skill-dir>/scripts/` |
| ImageMagick / `convert` | `convert --version` |
| FFmpeg / `ffmpeg` | `ffmpeg -version` |

### Decision matrix

| Primary workflow | Available | Action |
|---|---|---|
| Skill's primary approach + its dependencies | ✅ All present | Use primary workflow as documented in the skill |
| Skill's primary approach has missing deps | ❌ but skill documents a fallback | Use the **documented fallback**, tell the user which approach you switched to and why |
| Skill's primary approach has missing deps | ❌ and skill documents no fallback | Report the missing dependency to the user. Ask: (a) install X, (b) try alternative Y, or (c) suggest a different plan. **Do NOT silently invent a replacement.** |

**Golden rule: read before you write.** If the skill has a SKILL.md header note, dependency check step, or setup section — read and run it first. Do not skip setup steps.

---

## 3. Fallback Strategy

When a primary workflow is unavailable:

1. **Check the skill's SKILL.md for a documented fallback.** If present, use it verbatim.
2. **Check if an equivalent built-in tool/skill is available.** For example:
   - No image-generation skill → use `python-pptx` for text-based slides (all text editable, often the better choice for reports anyway).
   - No `officecli` → use `python-pptx` / `python-docx` / `openpyxl` for Office documents.
   - No dedicated PDF skill → check if `pypdf` / `reportlab` Python packages are available.
3. **No clear alternative → tell the user explicitly.** Say:
   > "I need X to do Y, but X is not available. Options: (a) install X, (b) use alternative Z that I have available, or (c) tell me another plan."

**Never silently switch approaches without telling the user what changed and why.** A one-sentence note is required.

---

## 4. File Management Discipline

- Do **not** create files outside the session root's `./outputs/` unless they are part of the project's own code structure (source files, tests, configs).
- **Do NOT create `outputs/` or `workspace/` directories inside skill folders** (`.claude/skills/*/`, `.codex/skills/*/`, `.gemini/skills/*/`). Skill folders are read-only resources. See §0.
- Do **not** create README / documentation / `.md` files unless the user explicitly asked.
- One-off helper scripts (Python, bash) go in the session root's `./workspace/`. If you write a one-off helper, consider whether it's worth keeping — otherwise, flag it as "helper, can be deleted after the run" in the output.
- Before claiming completion, verify the deliverable exists at the expected path (session root's `./outputs/`) with non-zero size.

---

## 5. Skill Usage Rules

When a skill is loaded and active:

1. **Follow the documented workflow.** Do not skip steps. Do not re-order steps unless the document explicitly allows it.
2. **Hard rules / Non-negotiable / MUST sections are mandatory.** If the skill has a section with these keywords, every item in that section is a deliverable requirement.
3. **Verify output quality gates.** If the skill defines a "QA" / "Delivery Gate" / "Visual floor" check, run it. Do not declare done while any gate fails.
4. **Inherit cross-skill rules.** If skill B says it "inherits from skill A", rules from A apply to B too. Read A first.

---

## 6. Communication Rules

- **When switching workflows, state the change.** Example: _"Note: image-generation skill is not available, so I'm using `python-pptx` to build the deck programmatically. Result: all text will be fully editable (copy/search) with consistent typography."_
- **When output differs from expectations, explain why.** Example: _"This deck is built with native shapes instead of AI images so your team can edit bullet text directly in PowerPoint."_
- **After completing a task, confirm the output path.** Example: _"File created at: `./outputs/project-management-best-practices.pptx`"_
- **If something fails and you retried, say how many attempts you made and what changed between them.** Transparency beats surprise.

---

## 7. Shell & Script Hygiene

- **Shell quoting:** When passing values with `$`, `!!`, `#`, spaces, or special characters into a shell command, **single-quote** the value to prevent expansion. In Python `subprocess.run([...])` list form, quoting is not needed.
- **Multi-line Python logic:** When you need more than ~3 lines of Python to do a job, write it to a script file (under `./workspace/` for helpers) instead of cramming it into `python -c "..."`. Quoting hell is avoidable.
- **Incremental execution:** Run and verify one structural step at a time. A 50-command script that fails at step 3 cascades silently. After create-slide / add-chart / add-connector, read back before stacking more.
- **Never inline secrets or API keys.** Pull them from environment variables or user-provided secure sources. Mask them in logs: `sk-...abcd` not the full key.

---

## Quick self-check before declaring a task done

1. [ ] Deliverable file is under the session root's `./outputs/` (NOT inside `.claude/skills/*/outputs/`).
2. [ ] Deliverable file exists with non-zero size (`ls -la ./outputs/<filename>` confirmed from session root).
3. [ ] No `outputs/` or `workspace/` directories were created inside skill folders.
4. [ ] Dependencies I used were confirmed available via a probe — not guessed.
5. [ ] If I switched from a primary workflow to a fallback, the user was told.
6. [ ] Any loaded skill's hard rules / delivery gates were run and passed.
