# -*- coding: utf-8 -*-
"""RunJam 商业 PPT 生成脚本 · 输出 16:9 PPTX"""
from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.shapes import MSO_SHAPE
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR

# === 品牌色 ===
C_BG          = RGBColor(0x0F, 0x12, 0x1A)
C_BG_SOFT     = RGBColor(0x1A, 0x1E, 0x2A)
C_BG_CARD     = RGBColor(0x22, 0x27, 0x36)
C_BORDER      = RGBColor(0x33, 0x39, 0x4A)
C_TEXT        = RGBColor(0xF1, 0xF5, 0xF9)
C_TEXT_DIM    = RGBColor(0x94, 0xA3, 0xB8)
C_TEXT_MUTED  = RGBColor(0x64, 0x74, 0x8B)
C_GREEN       = RGBColor(0x42, 0xB8, 0x83)
C_ORANGE      = RGBColor(0xFF, 0xA9, 0x4D)
C_RED         = RGBColor(0xCE, 0x42, 0x2B)
C_BLUE        = RGBColor(0x63, 0x66, 0xF1)
C_PURPLE      = RGBColor(0x8B, 0x5C, 0xF6)
C_AMBER       = RGBColor(0xF5, 0x9E, 0x0B)
C_PINK        = RGBColor(0xEC, 0x48, 0x99)
C_OK          = RGBColor(0x22, 0xC5, 0x5E)
C_WARN        = RGBColor(0xF5, 0x9E, 0x0B)
C_NO          = RGBColor(0xEF, 0x44, 0x44)

def set_bg(slide, c): slide.background.fill.solid(); slide.background.fill.fore_color.rgb = c

def add_rect(slide, x, y, w, h, fill=None, line=None, line_w=None, corner=None):
    shp = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, x, y, w, h)
    shp.shadow.inherit = False
    if corner is not None:
        try: shp.adjustments[0] = corner
        except Exception: pass
    if fill is None: shp.fill.background()
    else: shp.fill.solid(); shp.fill.fore_color.rgb = fill
    if line is None: shp.line.fill.background()
    else:
        shp.line.color.rgb = line
        if line_w is not None: shp.line.width = line_w
    return shp

def add_text(slide, x, y, w, h, text, *, size=18, bold=False, color=C_TEXT,
             align="left", anchor="middle", font="Microsoft YaHei"):
    tb = slide.shapes.add_textbox(x, y, w, h)
    tf = tb.text_frame
    tf.word_wrap = True
    for m in ("margin_left","margin_right","margin_top","margin_bottom"): setattr(tf, m, Emu(0))
    tf.vertical_anchor = {"top":MSO_ANCHOR.TOP,"middle":MSO_ANCHOR.MIDDLE,"bottom":MSO_ANCHOR.BOTTOM}[anchor]
    lines = text.split("\n") if isinstance(text, str) else text
    for i, line in enumerate(lines):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        p.alignment = {"left":PP_ALIGN.LEFT,"center":PP_ALIGN.CENTER,"right":PP_ALIGN.RIGHT}[align]
        r = p.add_run(); r.text = line
        r.font.name = font; r.font.size = Pt(size); r.font.bold = bold; r.font.color.rgb = color
    return tb

def add_multi(slide, x, y, w, h, parts, *, align="left", anchor="top"):
    tb = slide.shapes.add_textbox(x, y, w, h); tf = tb.text_frame; tf.word_wrap = True
    for m in ("margin_left","margin_right","margin_top","margin_bottom"): setattr(tf, m, Emu(0))
    tf.vertical_anchor = {"top":MSO_ANCHOR.TOP,"middle":MSO_ANCHOR.MIDDLE,"bottom":MSO_ANCHOR.BOTTOM}[anchor]
    al = {"left":PP_ALIGN.LEFT,"center":PP_ALIGN.CENTER,"right":PP_ALIGN.RIGHT}[align]
    cur = tf.paragraphs[0]; cur.alignment = al; first = True
    for part in parts:
        if part.get("newline_before") and not first: cur = tf.add_paragraph(); cur.alignment = al
        first = False
        r = cur.add_run(); r.text = part["text"]
        r.font.name = "Microsoft YaHei"
        r.font.size = Pt(part.get("size", 18))
        r.font.bold = part.get("bold", False)
        r.font.color.rgb = part.get("color", C_TEXT)
    return tb

def footer(slide, n, total):
    add_rect(slide, Inches(0.5), Inches(7.1), Inches(0.18), Inches(0.18), fill=C_GREEN, corner=0.5)
    add_text(slide, Inches(0.75), Inches(7.05), Inches(4), Inches(0.3),
             "RunJam · 一个桌面, 所有 AI Agent, 零锁定", size=10, color=C_TEXT_MUTED, anchor="middle")
    add_text(slide, Inches(12.6), Inches(7.1), Inches(0.6), Inches(0.3),
             f"{n} / {total}", size=10, color=C_TEXT_MUTED, align="right", anchor="middle")

# === 初始化 Presentation ===
prs = Presentation()
prs.slide_width  = Inches(13.333)
prs.slide_height = Inches(7.5)
BLANK = prs.slide_layouts[6]
TOTAL = 12

# ============================================================
# Slide 1 — 封面
# ============================================================
s = prs.slides.add_slide(BLANK); set_bg(s, C_BG)
# 装饰色块
for x,y,w,c in [(8.5,-1.5,6.5,C_GREEN),(9.5,-1.0,5.5,C_BG),(10.3,-0.5,4.5,C_BLUE),(10.8,-0.1,4.0,C_BG)]:
    add_rect(s, Inches(x), Inches(y), Inches(w), Inches(w), fill=c, corner=0.5)
# 三色 logo
for i,(dx,dy,c) in enumerate([(1.2,1.1,C_GREEN),(1.5,1.4,C_ORANGE),(1.8,1.7,C_RED),(2.1,2.0,C_BLUE)]):
    add_rect(s, Inches(dx), Inches(dy), Inches(0.7), Inches(0.7), fill=c, corner=0.18)
add_text(s, Inches(1.2), Inches(2.7), Inches(10), Inches(0.5), "RUNJAM", size=20, bold=True, color=C_GREEN)
add_multi(s, Inches(1.2), Inches(3.2), Inches(11.5), Inches(2.4), [
    {"text":"一个桌面, ","size":56,"bold":True,"color":C_TEXT},
    {"text":"所有 AI Agent, ","size":56,"bold":True,"color":C_GREEN},
    {"text":"零锁定。","size":56,"bold":True,"color":C_ORANGE},
])
add_text(s, Inches(1.2), Inches(4.85), Inches(11.5), Inches(0.5),
         "本地优先 · 多 Agent · 多模型 · 多项目 · 统一管理", size=22, color=C_TEXT_DIM)
add_rect(s, Inches(1.2), Inches(5.7), Inches(0.08), Inches(1.2), fill=C_GREEN, corner=1.0)
add_text(s, Inches(1.45), Inches(5.7), Inches(10), Inches(0.45),
         "Claude Code  ×  Codex CLI  ×  Gemini CLI", size=18, bold=True, color=C_GREEN)
add_text(s, Inches(1.45), Inches(6.15), Inches(10.5), Inches(0.6),
         "一次配置, 所有 Agent 都能用任意模型 —— 无需 ACP 改造, 无需逐个改配置, 不上云, 不绑死。",
         size=15, color=C_TEXT_DIM)
add_text(s, Inches(1.2), Inches(6.95), Inches(11), Inches(0.3),
         "Tauri 2  ·  Rust  ·  Vue 3  ·  MIT License", size=11, color=C_TEXT_MUTED)

# ============================================================
# Slide 2 — 概览
# ============================================================
s = prs.slides.add_slide(BLANK); set_bg(s, C_BG)
add_text(s, Inches(0.7), Inches(0.5), Inches(4), Inches(0.3), "01 · 概览", size=12, bold=True, color=C_GREEN)
add_text(s, Inches(0.7), Inches(0.85), Inches(12), Inches(0.7), "RunJam 是什么?", size=40, bold=True, color=C_TEXT)
add_text(s, Inches(0.7), Inches(1.6), Inches(12), Inches(0.5),
         "不是 AI, 不是 IDE, 不是云端 IDE —— RunJam 是 AI Agent 桌面管理器。",
         size=18, color=C_TEXT_DIM)
add_rect(s, Inches(0.7), Inches(2.5), Inches(11.9), Inches(2.0),
         fill=C_BG_SOFT, line=C_BORDER, line_w=Pt(1), corner=0.04)
add_rect(s, Inches(0.7), Inches(2.5), Inches(0.15), Inches(2.0), fill=C_GREEN, corner=1.0)
add_multi(s, Inches(1.1), Inches(2.75), Inches(11.4), Inches(1.6), [
    {"text":"把 ","size":28,"color":C_TEXT},
    {"text":" Claude Code ","size":28,"bold":True,"color":C_ORANGE},
    {"text":"、","size":28,"color":C_TEXT},
    {"text":"Codex CLI","size":28,"bold":True,"color":C_RED},
    {"text":"、","size":28,"color":C_TEXT},
    {"text":"Gemini CLI","size":28,"bold":True,"color":C_BLUE},
    {"text":" 装进一个桌面。","size":28,"bold":True,"color":C_TEXT,"newline_before":True},
    {"text":"通过内置协议代理, ","size":22,"color":C_TEXT_DIM,"newline_before":True},
    {"text":"Anthropic ↔ OpenAI ↔ Gemini","size":22,"bold":True,"color":C_GREEN},
    {"text":" 实时互转 —— 任意 Agent 用任意模型。","size":22,"color":C_TEXT_DIM},
])
# 4 张卖点卡
cards = [("🪟","统一窗口","聊天 / 文件树 / 编辑器 / 终端, 一窗搞定",C_GREEN),
         ("🔌","任意模型","一次配置, 全 Agent 同步, 协议自动转换",C_ORANGE),
         ("🛠️","零改造","通过原生 CLI stdin/stdout 驱动 Agent",C_RED),
         ("🔒","本地优先","数据全在 ~/.runjam/, 零遥测, 零云同步",C_BLUE)]
cw,ch,gap = 2.85,2.0,0.18
x0 = (13.333-4*cw-3*gap)/2
for i,(icon,title,sub,c) in enumerate(cards):
    x = x0+i*(cw+gap)
    add_rect(s, Inches(x), Inches(5.0), Inches(cw), Inches(ch), fill=C_BG_CARD, line=C_BORDER, line_w=Pt(0.75), corner=0.08)
    add_rect(s, Inches(x), Inches(5.0), Inches(cw), Inches(0.08), fill=c)
    add_text(s, Inches(x+0.2), Inches(5.25), Inches(0.8), Inches(0.6), icon, size=28, color=c)
    add_text(s, Inches(x+0.2), Inches(5.85), Inches(cw-0.4), Inches(0.45), title, size=18, bold=True, color=C_TEXT)
    add_text(s, Inches(x+0.2), Inches(6.3), Inches(cw-0.4), Inches(0.6), sub, size=11, color=C_TEXT_DIM)
footer(s, 2, TOTAL)

# ============================================================
# Slide 3 — 8 大痛点
# ============================================================
s = prs.slides.add_slide(BLANK); set_bg(s, C_BG)
add_text(s, Inches(0.7), Inches(0.5), Inches(4), Inches(0.3), "02 · 市场痛点", size=12, bold=True, color=C_RED)
add_text(s, Inches(0.7), Inches(0.85), Inches(12), Inches(0.7),
         "AI Agent 开发者的 8 大日常崩溃瞬间", size=36, bold=True, color=C_TEXT)
add_text(s, Inches(0.7), Inches(1.6), Inches(12), Inches(0.4),
         "如果以下场景你至少中过一条, RunJam 就是为你做的。", size=16, color=C_TEXT_DIM)
pains = [("1","终端动物园","五个终端标签页, 各跑不同 Agent。Kill 错一个, 会话全丢。",C_RED),
         ("2","模型不互通","同一 prompt 测 Claude / GPT / 本地 Qwen? 三套配置挨个改。",C_ORANGE),
         ("3","协议高墙","Claude / Codex / Gemini 各讲各的协议。换模型得钻进 Agent 改。",C_AMBER),
         ("4","Token 烧钱","每个轮次重发 system prompt。哪些命中缓存? 一无所知。",C_PINK),
         ("5","环境配置地狱","新项目 / 新 Agent / 新 Key / 新 PATH / 新版本冲突。",C_PURPLE),
         ("6","单一 Agent 锁","Cursor 包了 GPT-4, Copilot 包了 OpenAI。模型换 = 工作流迁移。",C_BLUE),
         ("7","云端数据焦虑","某些 \"AI IDE\" 把你的代码 / 配置 / Prompt 发到厂商服务器。",C_RED),
         ("8","会话黑洞","关掉 IDE 会话就丢, 换台机器从头来。",C_GREEN)]
cw,ch,gx,gy = 6.1,1.05,0.18,0.18
for i,(num,title,desc,c) in enumerate(pains):
    col,row = i%2, i//2
    x,y = 0.7+col*(cw+gx), 2.2+row*(ch+gy)
    add_rect(s, Inches(x), Inches(y), Inches(cw), Inches(ch), fill=C_BG_CARD, line=C_BORDER, line_w=Pt(0.75), corner=0.05)
    add_rect(s, Inches(x+0.2), Inches(y+0.18), Inches(0.7), Inches(0.7), fill=c, corner=0.5)
    add_text(s, Inches(x+0.2), Inches(y+0.18), Inches(0.7), Inches(0.7), num, size=24, bold=True, color=C_TEXT, align="center")
    add_text(s, Inches(x+1.05), Inches(y+0.18), Inches(cw-1.25), Inches(0.4), title, size=18, bold=True, color=C_TEXT)
    add_text(s, Inches(x+1.05), Inches(y+0.6), Inches(cw-1.25), Inches(0.4), desc, size=11, color=C_TEXT_DIM)
footer(s, 3, TOTAL)
