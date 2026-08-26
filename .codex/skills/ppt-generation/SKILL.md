---
name: ppt-generation
description: Use this skill when the user requests to generate, create, or make presentations (PPT/PPTX). Has TWO workflows: (1) Primary — AI-generated full-slide images composed via `scripts/generate.py`; (2) Fallback — `python-pptx` programmatic slides (all text editable, better for reports/project management). The fallback auto-activates when image-generation or the compose script is missing. Final PPTX always goes to `./outputs/`.
---
> **⚠️ This skill has TWO workflows. Always run the Dependency Check first and pick the right one — do NOT assume the image-based path works.**
> 1. **Primary (image-based):** Requires `image-generation` skill + `scripts/generate.py`. Generates full-slide images and composes them into PPTX.
> 2. **Fallback (python-pptx):** Use when image-generation or the compose script is missing. Creates slides programmatically with `python-pptx` — all text is editable, copyable, searchable. This is the BETTER choice for project management, reports, and data-heavy presentations.
>
> **Output path rule (from `runjam-defaults`):** The final `.pptx` file MUST be placed in `./outputs/`. Before starting: `mkdir -p ./outputs`. Never output to arbitrary directories.

# PPT Generation Skill
## Overview
This skill generates professional PowerPoint presentations. The primary workflow uses AI-generated images for each slide (composed via `scripts/generate.py`). When those dependencies are unavailable, the **fallback** workflow builds slides natively with `python-pptx` — all text remains editable, which is usually the preferred delivery for project-management decks, reports, and any content the user's team needs to modify.
## Core Capabilities
- Plan and structure multi-slide presentations with unified visual style
- Support multiple presentation styles: Business, Academic, Minimal, Apple Keynote, Creative
- Generate unique AI images for each slide using image-generation skill
- Maintain visual consistency by using previous slide as reference image
- Compose images into a professional PPTX file
## Presentation Styles
Choose one of the following styles when creating the presentation plan:
| Style | Description | Best For |
|-------|-------------|----------|
| **glassmorphism** | Frosted glass panels with blur effects, floating translucent cards, vibrant gradient backgrounds, depth through layering | Tech products, AI/SaaS demos, futuristic pitches |
| **dark-premium** | Rich black backgrounds (#0a0a0a), luminous accent colors, subtle glow effects, luxury brand aesthetic | Premium products, executive presentations, high-end brands |
| **gradient-modern** | Bold mesh gradients, fluid color transitions, contemporary typography, vibrant yet sophisticated | Startups, creative agencies, brand launches |
| **neo-brutalist** | Raw bold typography, high contrast, intentional "ugly" aesthetic, anti-design as design, Memphis-inspired | Edgy brands, Gen-Z targeting, disruptive startups |
| **3d-isometric** | Clean isometric illustrations, floating 3D elements, soft shadows, tech-forward aesthetic | Tech explainers, product features, SaaS presentations |
| **editorial** | Magazine-quality layouts, sophisticated typography hierarchy, dramatic photography, Vogue/Bloomberg aesthetic | Annual reports, luxury brands, thought leadership |
| **minimal-swiss** | Grid-based precision, Helvetica-inspired typography, bold use of negative space, timeless modernism | Architecture, design firms, premium consulting |
| **keynote** | Apple-inspired aesthetic with bold typography, dramatic imagery, high contrast, cinematic feel | Keynotes, product reveals, inspirational talks |

## Dependency Check (MUST RUN FIRST)

Before starting any workflow, check what's actually available:

1. Check if `../image-generation/SKILL.md` exists → image-based primary workflow possible?
2. Check if `./scripts/generate.py` exists → compose script available?
3. Check `python-pptx`: `python -c "import pptx" 2>&1` (needed for both compose and fallback)

**Decision matrix:**

| image-generation skill | scripts/generate.py | python-pptx | Workflow |
|---|---|---|---|
| ✅ present | ✅ present | ✅ installed | **Primary (image-based)** |
| ❌ missing | ❌ missing | ✅ installed | **Fallback (python-pptx)** — tell the user you switched and why |
| ❌ missing | ❌ missing | ❌ missing | Install `python-pptx` first: `pip install python-pptx`, then use Fallback |
| ⚠️ any mix | ⚠️ any mix | ✅ installed | Use **Fallback** — avoid partial image workflow; the compose chain breaks without ALL pieces |

**Note:** In most RunJam installations today, neither `image-generation` nor `scripts/generate.py` ship with the app. Assume Fallback unless you explicitly see both present.

## Workflow
### Step 1: Understand Requirements
When a user requests presentation generation, identify:
- Topic/subject: What is the presentation about
- Number of slides: How many slides are needed (default: 5-10)
- **Style**: business / academic / minimal / keynote / creative
- Aspect ratio: Standard (16:9) or classic (4:3)
- Content outline: Key points for each slide
- You don't need to check the folder under `.`
### Step 2: Create Presentation Plan
Create a JSON file in `./workspace/` with the presentation structure. **Important**: Include the `style` field to define the overall visual consistency.
```json
{
  "title": "Presentation Title",
  "style": "keynote",
  "style_guidelines": {
    "color_palette": "Deep black backgrounds, white text, single accent color (blue or orange)",
    "typography": "Bold sans-serif headlines, clean body text, dramatic size contrast",
    "imagery": "High-quality photography, full-bleed images, cinematic composition",
    "layout": "Generous whitespace, centered focus, minimal elements per slide"
  },
  "aspect_ratio": "16:9",
  "slides": [
    {
      "slide_number": 1,
      "type": "title",
      "title": "Main Title",
      "subtitle": "Subtitle or tagline",
      "visual_description": "Detailed description for image generation"
    },
    {
      "slide_number": 2,
      "type": "content",
      "title": "Slide Title",
      "key_points": ["Point 1", "Point 2", "Point 3"],
      "visual_description": "Detailed description for image generation"
    }
  ]
}
```
### Step 3: Generate Slide Images Sequentially
**IMPORTANT**: Generate slides **strictly one by one, in order**. Do NOT parallelize or batch image generation. Each slide depends on the previous slide's output as a reference image. Generating slides in parallel will break visual consistency and is not allowed.
1. Read the image-generation skill: `../image-generation/SKILL.md`
2. **For the FIRST slide (slide 1)**, create a prompt that establishes the visual style:
```json
{
  "prompt": "Professional presentation slide. [style_guidelines from plan]. Title: 'Your Title'. [visual_description]. This slide establishes the visual language for the entire presentation.",
  "style": "[Based on chosen style - e.g., Apple Keynote aesthetic, dramatic lighting, cinematic]",
  "composition": "Clean layout with clear text hierarchy, [style-specific composition]",
  "color_palette": "[From style_guidelines]",
  "typography": "[From style_guidelines]"
}
```
```bash
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/slide-01-prompt.json \
  --output-file ./outputs/slide-01.jpg \
  --aspect-ratio 16:9
```
3. **For subsequent slides (slide 2+)**, use the PREVIOUS slide as a reference image:
```json
{
  "prompt": "Professional presentation slide continuing the visual style from the reference image. Maintain the same color palette, typography style, and overall aesthetic. Title: 'Slide Title'. [visual_description]. Keep visual consistency with the reference.",
  "style": "Match the style of the reference image exactly",
  "composition": "Similar layout principles as reference, adapted for this content",
  "color_palette": "Same as reference image",
  "consistency_note": "This slide must look like it belongs in the same presentation as the reference image"
}
```
```bash
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/slide-02-prompt.json \
  --reference-images ./outputs/slide-01.jpg \
  --output-file ./outputs/slide-02.jpg \
  --aspect-ratio 16:9
```
4. **Continue for all remaining slides**, always referencing the previous slide:
```bash
# Slide 3 references slide 2
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/slide-03-prompt.json \
  --reference-images ./outputs/slide-02.jpg \
  --output-file ./outputs/slide-03.jpg \
  --aspect-ratio 16:9
# Slide 4 references slide 3
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/slide-04-prompt.json \
  --reference-images ./outputs/slide-03.jpg \
  --output-file ./outputs/slide-04.jpg \
  --aspect-ratio 16:9
```
### Step 4: Compose PPT
After all slide images are generated, call the composition script:
```bash
python scripts/generate.py \
  --plan-file ./workspace/presentation-plan.json \
  --slide-images ./outputs/slide-01.jpg ./outputs/slide-02.jpg ./outputs/slide-03.jpg \
  --output-file ./outputs/presentation.pptx
```
Parameters:
- `--plan-file`: Absolute path to the presentation plan JSON file (required)
- `--slide-images`: Absolute paths to slide images in order (required, space-separated)
- `--output-file`: Absolute path to output PPTX file (required)
[!NOTE]
Do NOT read the python file, just call it with the parameters.

## Fallback Workflow: python-pptx (programmatic slide creation)

Use this workflow when the `image-generation` skill OR `scripts/generate.py` is not available. This approach creates slides natively using `python-pptx`, resulting in real PowerPoint files where all text is editable, copyable, and searchable. For project management decks, status reports, training material, and data-heavy content this is usually the **better** deliverable — your user's team can edit slides directly.

### Fallback Step 0: Ensure dependencies + output dir

**⚠️ Run these from the SESSION WORKING DIRECTORY (session root), NOT from inside the skill folder.** If `pwd` contains `skills/`, `cd` up to the session root first.

```bash
pwd                          # MUST show session root, NOT .../skills/ppt-generation
mkdir -p ./outputs ./workspace
# Check python-pptx
python -c "import pptx" 2>&1
# If the above fails → install:
#   pip install python-pptx
```

**Do NOT create `outputs/` or `workspace/` inside `.claude/skills/ppt-generation/`.** That is the #1 mistake — skill folders are read-only. See `runjam-defaults` §0.

### Fallback Step 1: Write the build script

Create a Python script at `./workspace/build_<deck-name>_pptx.py` (in the **session root's** workspace, NOT the skill folder). Use this pattern:

```python
import os
from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN
from pptx.enum.shapes import MSO_SHAPE

# --- Path safety guard (from runjam-defaults §0) ---
# Ensure outputs land in the session working directory, NOT inside a skill folder.
# If cwd contains '.claude/skills' or '.codex/skills' or '.gemini/skills',
# walk up to the session root (parent of the .claude/.codex/.gemini dir).
_cwd = os.getcwd()
for _marker in ('.claude', '.codex', '.gemini'):
    _idx = _cwd.find(os.sep + _marker + os.sep + 'skills')
    if _idx != -1:
        os.chdir(_cwd[:_idx])
        break
SESSION_ROOT  = os.getcwd()
OUTPUTS_DIR   = os.path.join(SESSION_ROOT, 'outputs')
WORKSPACE_DIR = os.path.join(SESSION_ROOT, 'workspace')
os.makedirs(OUTPUTS_DIR, exist_ok=True)
os.makedirs(WORKSPACE_DIR, exist_ok=True)

# --- Configuration ---
OUTPUT_PATH = os.path.join(OUTPUTS_DIR, "project-management-best-practices.pptx")
ASPECT_W, ASPECT_H = Inches(13.333), Inches(7.5)  # 16:9

# Color palette (pick one consistent theme; see presets below)
BG          = RGBColor(0x0F, 0x17, 0x2A)  # deep navy bg
ACCENT      = RGBColor(0x3B, 0x82, 0xF6)  # primary blue
ACCENT_2    = RGBColor(0x10, 0xB9, 0x81)  # secondary green
TITLE_COLOR = RGBColor(0xFF, 0xFF, 0xFF)
BODY_COLOR  = RGBColor(0xCB, 0xD5, 0xE1)
CARD_BG     = RGBColor(0x1E, 0x29, 0x3B)

# Palette presets (swap BG/ACCENT*/TITLE*/BODY*/CARD_BG as needed):
#   Navy executive:  BG=0F172A  ACCENT=3B82F6  ACCENT2=10B981  TITLE=FFFFFF  BODY=CBD5E1  CARD=1E293B
#   Forest & moss:   BG=0E1F12  ACCENT=22C55E  ACCENT2=F59E0B  TITLE=FFFFFF  BODY=D1FAE5  CARD=18351C
#   Warm terracotta: BG=1F120B  ACCENT=EA580C  ACCENT2=DC2626  TITLE=FFF7ED  BODY=FED7AA  CARD=2E1C12
#   Charcoal minimal: BG=1C1C1E ACCENT=0A84FF  ACCENT2=8E8E93  TITLE=FFFFFF  BODY=E5E5EA  CARD=2C2C2E
#   Light corporate:  BG=FFFFFF  ACCENT=1D4ED8  ACCENT2=047857  TITLE=0F172A  BODY=475569  CARD=F1F5F9

# --- Setup presentation ---
prs = Presentation()
prs.slide_width  = ASPECT_W
prs.slide_height = ASPECT_H
SLIDE_LAYOUT_BLANK = prs.slide_layouts[6]  # 6 = blank

def add_slide(bg_color=BG):
    s = prs.slides.add_slide(SLIDE_LAYOUT_BLANK)
    b = s.background.fill
    b.solid()
    b.fore_color.rgb = bg_color
    return s

def add_text(slide, x_in, y_in, w_in, h_in, text, *,
             font_size=18, bold=False, color=BODY_COLOR,
             align=PP_ALIGN.LEFT, word_wrap=True):
    tb = slide.shapes.add_textbox(Inches(x_in), Inches(y_in),
                                  Inches(w_in), Inches(h_in))
    tf = tb.text_frame
    tf.word_wrap = word_wrap
    p = tf.paragraphs[0]
    p.text = text
    p.font.size = Pt(font_size)
    p.font.bold = bold
    p.font.color.rgb = color
    p.alignment = align
    return tb

def add_rect(slide, x_in, y_in, w_in, h_in, *, fill_color=CARD_BG, line_color=None):
    shp = slide.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE,
                                 Inches(x_in), Inches(y_in),
                                 Inches(w_in), Inches(h_in))
    shp.fill.solid()
    shp.fill.fore_color.rgb = fill_color
    if line_color is None:
        shp.line.fill.background()
    else:
        shp.line.color.rgb = line_color
    shp.shadow.inherit = False
    return shp

def add_accent_bar(slide, x_in, y_in, w_in=0.08, h_in=0.5, *, color=ACCENT):
    shp = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                 Inches(x_in), Inches(y_in),
                                 Inches(w_in), Inches(h_in))
    shp.fill.solid()
    shp.fill.fore_color.rgb = color
    shp.line.fill.background()
    shp.shadow.inherit = False
    return shp

# --- Style tier sizes (HARD RULES from runjam-defaults) ---
# Title ≥ 36pt bold, body ≥ 18pt, title ≥ 2x body size.
# Edges: ≥ 0.5in (≈1.27cm) margin. Negative space ≥ 20%.

# =========================================================
# SLIDE 1 — Title / Cover
# =========================================================
s = add_slide()
# Left accent band
add_accent_bar(s, 0.8, 2.6, w_in=0.10, h_in=2.4, color=ACCENT)
# Title
add_text(s, 1.2, 2.6, 11.0, 1.4,
         "Project Management Best Practices",
         font_size=44, bold=True, color=TITLE_COLOR)
# Subtitle
add_text(s, 1.2, 4.1, 11.0, 0.8,
         "Deliver on Time, on Scope, on Budget — Every Time",
         font_size=22, color=ACCENT)
# Footer meta row
add_text(s, 1.2, 6.3, 8.0, 0.4,
         "Internal Playbook  •  Q3 2026",
         font_size=12, color=BODY_COLOR)

# =========================================================
# SLIDE 2 — Agenda / Contents
# =========================================================
s = add_slide()
add_accent_bar(s, 0.8, 0.9, w_in=0.10, h_in=0.55, color=ACCENT)
add_text(s, 1.2, 0.8, 11.0, 0.8,
         "Agenda", font_size=40, bold=True, color=TITLE_COLOR)

items = [
    ("01", "Foundations — Goals, Scope, Stakeholders"),
    ("02", "Planning — WBS, Schedules, Risks, Estimates"),
    ("03", "Execution — Standups, Tracking, Communication"),
    ("04", "Controlling — Variance, Change Control, Quality"),
    ("05", "Closing — Handover, Retrospectives, Lessons"),
    ("06", "Common Failures and How to Avoid Them"),
]
y = 2.2
for num, label in items:
    add_rect(s, 1.0, y, 11.3, 0.62, fill_color=CARD_BG)
    add_text(s, 1.2, y+0.08, 0.8, 0.5,
             num, font_size=20, bold=True, color=ACCENT)
    add_text(s, 2.2, y+0.12, 9.9, 0.5,
             label, font_size=18, color=BODY_COLOR)
    y += 0.74

# =========================================================
# SLIDE 3+ — Build the rest iteratively
# =========================================================
# Pattern for a content slide:
#   s = add_slide()
#   add_accent_bar(s, 0.8, 0.9, w_in=0.10, h_in=0.55)
#   add_text(s, 1.2, 0.8, 11.0, 0.8, "Section Title", font_size=40, bold=True, color=TITLE_COLOR)
#   # Add cards / bullets / KPI numbers using add_rect + add_text
#
# Pattern for a bullet card (3 cards across):
#   col_w, card_h = 3.84, 3.6
#   gap, left0, top0 = 0.3, 1.0, 2.0
#   cards = [("Title A", ["p1","p2","p3"]), ("Title B", ["..."]), ("Title C", ["..."])]
#   for i, (t, bullets) in enumerate(cards):
#       x = left0 + i*(col_w + gap)
#       add_rect(s, x, top0, col_w, card_h)
#       add_text(s, x+0.25, top0+0.2, col_w-0.5, 0.6, t, font_size=22, bold=True, color=TITLE_COLOR)
#       by = top0 + 1.0
#       for b in bullets:
#           add_text(s, x+0.25, by, col_w-0.5, 0.45, "•  " + b, font_size=16, color=BODY_COLOR)
#           by += 0.5

# --- Save (ALWAYS under ./outputs/) ---
prs.save(OUTPUT_PATH)
print(f"✅ Saved: {OUTPUT_PATH}")
print(f"   Slides: {len(prs.slides)}")
```

### Fallback Step 2: Execute and verify

```bash
python ./workspace/build_<deck-name>_pptx.py
# Verify output exists with non-zero size
ls -la ./outputs/*.pptx
```

### Fallback Step 3: Hard rules (from `runjam-defaults`)

Every deck produced via Fallback MUST satisfy:

- **One idea per slide.** If a slide needs a second title to explain its scope → split.
- **Type hierarchy set explicitly** (no theme-default drift): slide title **≥ 36pt bold**, body text **≥ 18pt**, title size ≥ 2× body size. Left-align body; center only titles and hero KPI numbers.
- **Contrast floor.** Light text on dark background (or dark text on light) — never dark-on-dark or light-on-light. When in doubt, use the palette presets above; they are vetted.
- **Each content slide carries a non-text visual that informs.** A card layout, a KPI rectangle with an accent bar, a bullet card grid, or an icon-like shape — not just a wall of bullets.
- **Margins + negative space.** Edge margin ≥ 0.5in (≈1.27cm) on all sides. Inter-block gap ≥ 0.3in (≈0.76cm). ~20% negative space; don't pack until it bursts.
- **Speaker notes** on every non-cover slide. Add via the plan JSON (carry narration) and write them as a text paragraph in the build script, or add them with `python-pptx` slide notes API after creating the slide.

## Complete Example: Glassmorphism Style (最现代前卫)
User request: "Create a presentation about AI product launch"
### Step 1: Create presentation plan
Create `./workspace/ai-product-plan.json`:
```json
{
  "title": "Introducing Nova AI",
  "style": "glassmorphism",
  "style_guidelines": {
    "color_palette": "Vibrant purple-to-cyan gradient background (#667eea→#00d4ff), frosted glass panels with 15-20% white opacity, electric accents",
    "typography": "SF Pro Display style, bold 700 weight white titles with subtle text-shadow, clean 400 weight body text, excellent contrast on glass",
    "imagery": "Abstract 3D glass spheres, floating translucent geometric shapes, soft luminous orbs, depth through layered transparency",
    "layout": "Centered frosted glass cards with 32px rounded corners, 48-64px padding, floating above gradient, layered depth with soft shadows",
    "effects": "Backdrop blur 20-40px on glass panels, subtle white border glow, soft colored shadows matching gradient, light refraction effects",
    "visual_language": "Apple Vision Pro / visionOS aesthetic, premium depth through transparency, futuristic yet approachable, 2024 design trends"
  },
  "aspect_ratio": "16:9",
  "slides": [
    {
      "slide_number": 1,
      "type": "title",
      "title": "Introducing Nova AI",
      "subtitle": "Intelligence, Reimagined",
      "visual_description": "Stunning gradient background flowing from deep purple (#667eea) through magenta to cyan (#00d4ff). Center: large frosted glass panel with strong backdrop blur, containing bold white title 'Introducing Nova AI' and lighter subtitle. Floating 3D glass spheres and abstract shapes around the card creating depth. Soft glow emanating from behind the glass panel. Premium visionOS aesthetic. The glass card has subtle white border (1px rgba 255,255,255,0.3) and soft purple-tinted shadow."
    },
    {
      "slide_number": 2,
      "type": "content",
      "title": "Why Nova?",
      "key_points": ["10x faster processing", "Human-like understanding", "Enterprise-grade security"],
      "visual_description": "Same purple-cyan gradient background. Left side: floating frosted glass card with title 'Why Nova?' in bold white, three key points below with subtle glass pill badges. Right side: abstract 3D visualization of neural network as interconnected glass nodes with soft glow. Floating translucent geometric shapes (icosahedrons, tori) adding depth. Consistent glassmorphism aesthetic with previous slide."
    },
    {
      "slide_number": 3,
      "type": "content",
      "title": "How It Works",
      "key_points": ["Natural language input", "Multi-modal processing", "Instant insights"],
      "visual_description": "Gradient background consistent with previous slides. Central composition: three stacked frosted glass cards at slight angles showing the workflow steps, connected by soft glowing lines. Each card has an abstract icon. Floating glass orbs and light particles around the composition. Title 'How It Works' in bold white at top. Depth created through card layering and transparency."
    },
    {
      "slide_number": 4,
      "type": "content",
      "title": "Built for Scale",
      "key_points": ["1M+ concurrent users", "99.99% uptime", "Global infrastructure"],
      "visual_description": "Same gradient background. Asymmetric layout: right side features large frosted glass panel with metrics displayed in bold typography. Left side: abstract 3D globe made of glass panels and connection lines, representing global scale. Floating data visualization elements as small glass cards with numbers. Soft ambient glow throughout. Premium tech aesthetic."
    },
    {
      "slide_number": 5,
      "type": "conclusion",
      "title": "The Future Starts Now",
      "subtitle": "Join the waitlist",
      "visual_description": "Dramatic finale slide. Gradient background with slightly increased vibrancy. Central frosted glass card with bold title 'The Future Starts Now' and call-to-action subtitle. Behind the card: burst of soft light rays and floating glass particles creating celebration effect. Multiple layered glass shapes creating depth. The most visually impactful slide while maintaining style consistency."
    }
  ]
}
```
### Step 2: Read image-generation skill
Read `../image-generation/SKILL.md` to understand how to generate images.
### Step 3: Generate slide images sequentially with reference chaining
**Slide 1 - Title (establishes the visual language):**
Create `./workspace/nova-slide-01.json`:
```json
{
  "prompt": "Ultra-premium presentation title slide with glassmorphism design. Background: smooth flowing gradient from deep purple (#667eea) through magenta (#f093fb) to cyan (#00d4ff), soft and vibrant. Center: large frosted glass panel with strong backdrop blur effect, rounded corners 32px, containing bold white sans-serif title 'Introducing Nova AI' (72pt, SF Pro Display style, font-weight 700) with subtle text shadow, subtitle 'Intelligence, Reimagined' below in lighter weight. The glass panel has subtle white border (1px rgba 255,255,255,0.25) and soft purple-tinted drop shadow. Floating around the card: 3D glass spheres with refraction, translucent geometric shapes (icosahedrons, abstract blobs), creating depth and dimension. Soft luminous glow emanating from behind the glass panel. Small floating particles of light. Apple Vision Pro / visionOS UI aesthetic. Professional presentation slide, 16:9 aspect ratio. Hyper-modern, premium tech product launch feel.",
  "style": "Glassmorphism, visionOS aesthetic, Apple Vision Pro UI style, premium tech, 2024 design trends",
  "composition": "Centered glass card as focal point, floating 3D elements creating depth at edges, 40% negative space, clear visual hierarchy",
  "lighting": "Soft ambient glow from gradient, light refraction through glass elements, subtle rim lighting on 3D shapes",
  "color_palette": "Purple gradient #667eea, magenta #f093fb, cyan #00d4ff, frosted white rgba(255,255,255,0.15), pure white text #ffffff",
  "effects": "Backdrop blur on glass panels, soft drop shadows with color tint, light refraction, subtle noise texture on glass, floating particles"
}
```
```bash
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/nova-slide-01.json \
  --output-file ./outputs/nova-slide-01.jpg \
  --aspect-ratio 16:9
```
**Slide 2 - Content (MUST reference slide 1 for consistency):**
Create `./workspace/nova-slide-02.json`:
```json
{
  "prompt": "Presentation slide continuing EXACT visual style from reference image. SAME purple-to-cyan gradient background, SAME glassmorphism aesthetic, SAME typography style. Left side: frosted glass card with backdrop blur containing title 'Why Nova?' in bold white (matching reference font style), three feature points as subtle glass pill badges below. Right side: abstract 3D neural network visualization made of interconnected glass nodes with soft cyan glow, floating in space. Floating translucent geometric shapes (matching style from reference) adding depth. The frosted glass has identical treatment: white border, purple-tinted shadow, same blur intensity. CRITICAL: This slide must look like it belongs in the exact same presentation as the reference image - same colors, same glass treatment, same overall aesthetic.",
  "style": "MATCH REFERENCE EXACTLY - Glassmorphism, visionOS aesthetic, same visual language",
  "composition": "Asymmetric split: glass card left (40%), 3D visualization right (40%), breathing room between elements",
  "color_palette": "EXACTLY match reference: purple #667eea, cyan #00d4ff gradient, same frosted white treatment, same text white",
  "consistency_note": "CRITICAL: Must be visually identical in style to reference image. Same gradient colors, same glass blur intensity, same shadow treatment, same typography weight and style. Viewer should immediately recognize this as the same presentation."
}
```
```bash
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/nova-slide-02.json \
  --reference-images ./outputs/nova-slide-01.jpg \
  --output-file ./outputs/nova-slide-02.jpg \
  --aspect-ratio 16:9
```
**Slides 3-5 - Continue the reference chaining:**
Follow the same pattern for the remaining slides, always using the immediately preceding slide as the reference image (`--reference-images ./outputs/nova-slide-02.jpg` for slide 3, `./outputs/nova-slide-03.jpg` for slide 4, and so on). Each prompt must emphasize matching the visual language established by slide 1.
```bash
# Slide 3 references slide 2
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/nova-slide-03.json \
  --reference-images ./outputs/nova-slide-02.jpg \
  --output-file ./outputs/nova-slide-03.jpg \
  --aspect-ratio 16:9
# Slide 4 references slide 3
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/nova-slide-04.json \
  --reference-images ./outputs/nova-slide-03.jpg \
  --output-file ./outputs/nova-slide-04.jpg \
  --aspect-ratio 16:9
# Slide 5 references slide 4
python ../image-generation/scripts/generate.py \
  --prompt-file ./workspace/nova-slide-05.json \
  --reference-images ./outputs/nova-slide-04.jpg \
  --output-file ./outputs/nova-slide-05.jpg \
  --aspect-ratio 16:9
```
### Step 4: Compose PPT
After all 5 slide images are generated, compose them into the final PPTX file:
```bash
python scripts/generate.py \
  --plan-file ./workspace/ai-product-plan.json \
  --slide-images ./outputs/nova-slide-01.jpg ./outputs/nova-slide-02.jpg ./outputs/nova-slide-03.jpg ./outputs/nova-slide-04.jpg ./outputs/nova-slide-05.jpg \
  --output-file ./outputs/nova-ai-presentation.pptx
```
The final presentation `nova-ai-presentation.pptx` will be created in `./outputs/`.
