---
name: ppt-generation
description: Use this skill when the user requests to generate, create, or make presentations (PPT/PPTX). Creates visually rich slides by generating images for each slide and composing them into a PowerPoint file.
---
> **Note:** This skill depends on the `image-generation` skill for slide image generation. If it's not available, use any available image generation tool. The `scripts/generate.py` composes slide images into a PPTX file using python-pptx.

# PPT Generation Skill
## Overview
This skill generates professional PowerPoint presentations by creating AI-generated images for each slide and composing them into a PPTX file. The workflow includes planning the presentation structure with a consistent visual style, generating slide images sequentially (using the previous slide as a reference for style consistency), and assembling them into a final presentation.
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
