#!/usr/bin/env python3
"""Generate Kyara Player's built-in classic skin.

Original artwork drawn from code; only the sheet sizes and sprite coordinates follow
the classic skin format so that any .wsz skin is a drop-in replacement.

    python3 tools/gen_default_skin.py   # writes crates/skin/assets/default/*.png
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "crates/skin/assets/default"

# Palette
PANEL_HI = (58, 61, 78)
PANEL = (40, 42, 54)
PANEL_LO = (28, 29, 38)
EDGE = (12, 12, 18)
LCD_BG = (6, 10, 8)
LCD_FG = (46, 232, 106)
LCD_DIM = (18, 70, 38)
LABEL = (150, 154, 176)
BTN_HI = (92, 96, 120)
BTN = (66, 69, 88)
BTN_LO = (44, 46, 60)
GLYPH = (220, 222, 236)
ACCENT = (46, 232, 106)
TITLE_ACTIVE = ((70, 74, 120), (38, 40, 70))
TITLE_INACTIVE = ((62, 63, 76), (40, 41, 50))

# 3x5 pixel font (original). Rows top to bottom, "#" = ink.
GLYPHS = {
    "a": ".#.|#.#|###|#.#|#.#", "b": "##.|#.#|##.|#.#|##.", "c": ".##|#..|#..|#..|.##",
    "d": "##.|#.#|#.#|#.#|##.", "e": "###|#..|##.|#..|###", "f": "###|#..|##.|#..|#..",
    "g": ".##|#..|#.#|#.#|.##", "h": "#.#|#.#|###|#.#|#.#", "i": "###|.#.|.#.|.#.|###",
    "j": "..#|..#|..#|#.#|.#.", "k": "#.#|#.#|##.|#.#|#.#", "l": "#..|#..|#..|#..|###",
    "m": "#.#|###|#.#|#.#|#.#", "n": "##.|#.#|#.#|#.#|#.#", "o": ".#.|#.#|#.#|#.#|.#.",
    "p": "##.|#.#|##.|#..|#..", "q": ".#.|#.#|#.#|##.|.##", "r": "##.|#.#|##.|#.#|#.#",
    "s": ".##|#..|.#.|..#|##.", "t": "###|.#.|.#.|.#.|.#.", "u": "#.#|#.#|#.#|#.#|###",
    "v": "#.#|#.#|#.#|#.#|.#.", "w": "#.#|#.#|#.#|###|#.#", "x": "#.#|#.#|.#.|#.#|#.#",
    "y": "#.#|#.#|.#.|.#.|.#.", "z": "###|..#|.#.|#..|###",
    "0": "###|#.#|#.#|#.#|###", "1": ".#.|##.|.#.|.#.|###", "2": "##.|..#|.#.|#..|###",
    "3": "##.|..#|.#.|..#|##.", "4": "#.#|#.#|###|..#|..#", "5": "###|#..|##.|..#|##.",
    "6": ".##|#..|###|#.#|###", "7": "###|..#|.#.|.#.|.#.", "8": "###|#.#|###|#.#|###",
    "9": "###|#.#|###|..#|##.",
    '"': "#.#|#.#|...|...|...", "@": ".#.|#.#|##.|#..|.##", " ": "...|...|...|...|...",
    "…": "...|...|...|...|#.#", ".": "...|...|...|...|.#.", ":": "...|.#.|...|.#.|...",
    "(": "..#|.#.|.#.|.#.|..#", ")": "#..|.#.|.#.|.#.|#..", "-": "...|...|###|...|...",
    "'": ".#.|.#.|...|...|...", "!": ".#.|.#.|.#.|...|.#.", "_": "...|...|...|...|###",
    "+": "...|.#.|###|.#.|...", "\\": "#..|#..|.#.|..#|..#", "/": "..#|..#|.#.|#..|#..",
    "[": "##.|#..|#..|#..|##.", "]": ".##|..#|..#|..#|.##", "^": ".#.|#.#|...|...|...",
    "&": ".#.|#.#|.#.|#.#|.##", "%": "#.#|..#|.#.|#..|#.#", ",": "...|...|...|.#.|#..",
    "=": "...|###|...|###|...", "$": ".##|##.|.#.|.##|##.", "#": "#.#|###|#.#|###|#.#",
    "?": "##.|..#|.#.|...|.#.", "*": "...|#.#|.#.|#.#|...",
}
GLYPHS["å"] = GLYPHS["a"]
GLYPHS["ä"] = GLYPHS["a"]
GLYPHS["ö"] = GLYPHS["o"]

# Cell positions in text.bmp (row, column); 5x6 cells.
TEXT_LAYOUT = {}
for i, ch in enumerate("abcdefghijklmnopqrstuvwxyz\"@"):
    TEXT_LAYOUT[ch] = (0, i)
TEXT_LAYOUT[" "] = (0, 30)
for i, ch in enumerate("0123456789….:()-'!_+\\/[]^&%,=$#"):
    TEXT_LAYOUT[ch] = (1, i)
for i, ch in enumerate("åöä?*"):
    TEXT_LAYOUT[ch] = (2, i)


def glyph(d: ImageDraw.ImageDraw, x: int, y: int, ch: str, color, scale: int = 1):
    rows = GLYPHS.get(ch.lower(), GLYPHS[" "]).split("|")
    for gy, row in enumerate(rows):
        for gx, px in enumerate(row):
            if px == "#":
                d.rectangle(
                    [x + gx * scale, y + gy * scale, x + gx * scale + scale - 1, y + gy * scale + scale - 1],
                    fill=color,
                )


def text(d, x, y, s, color, spacing=4):
    for i, ch in enumerate(s):
        glyph(d, x + i * spacing, y, ch, color)


def bevel(d, box, hi, mid, lo):
    x0, y0, x1, y1 = box
    d.rectangle(box, fill=mid)
    d.line([x0, y0, x1, y0], fill=hi)
    d.line([x0, y0, x0, y1], fill=hi)
    d.line([x0, y1, x1, y1], fill=lo)
    d.line([x1, y0, x1, y1], fill=lo)


def inset(d, box, fill=LCD_BG):
    x0, y0, x1, y1 = box
    d.rectangle(box, fill=fill)
    d.line([x0, y0, x1, y0], fill=EDGE)
    d.line([x0, y0, x0, y1], fill=EDGE)
    d.line([x0, y1, x1, y1], fill=PANEL_HI)
    d.line([x1, y0, x1, y1], fill=PANEL_HI)


def vgradient(img, box, top, bottom):
    x0, y0, x1, y1 = box
    d = ImageDraw.Draw(img)
    h = max(1, y1 - y0)
    for y in range(y0, y1 + 1):
        t = (y - y0) / h
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
        d.line([x0, y, x1, y], fill=c)


def new(w, h, color=PANEL):
    img = Image.new("RGB", (w, h), color)
    return img, ImageDraw.Draw(img)


# ---------------------------------------------------------------- main.bmp
def main_bmp():
    img, d = new(275, 116)
    vgradient(img, (0, 0, 274, 115), PANEL_HI, PANEL_LO)
    d.rectangle([0, 0, 274, 115], outline=EDGE)
    # Display: clutter bar sits at x 10..17, LCD to its right.
    inset(d, (20, 23, 104, 63))
    # Marquee, kbps, khz.
    inset(d, (108, 24, 267, 36))
    inset(d, (108, 40, 128, 51))
    text(d, 131, 43, "kbps", LABEL)
    inset(d, (153, 40, 168, 51))
    text(d, 171, 43, "khz", LABEL)
    # Slider wells (drawn under volume/balance sprites) and window toggles area.
    inset(d, (106, 56, 176, 70), PANEL_LO)
    inset(d, (176, 56, 216, 70), PANEL_LO)
    inset(d, (218, 57, 266, 70), PANEL_LO)
    # Button tray.
    inset(d, (14, 86, 160, 107), PANEL_LO)
    inset(d, (162, 87, 240, 105), PANEL_LO)
    # Emblem: a small "K" in the about box.
    inset(d, (251, 89, 267, 107), LCD_BG)
    glyph(d, 253, 91, "k", ACCENT, scale=3)
    return img


# ---------------------------------------------------------------- titlebar.bmp
def title_strip(img, d, y, colors, label_color):
    vgradient(img, (27, y, 301, y + 13), *colors)
    d.rectangle([27, y, 301, y + 13], outline=EDGE)
    # Grip lines either side of the title.
    for gx in (50, 196):
        for i in range(0, 44, 2):
            d.line([gx + i, y + 4, gx + i, y + 9], fill=colors[0] if i % 4 else PANEL_LO)
    text(d, 140, y + 4, "kyara player", label_color)


def tb_button(d, x, y, kind, pressed):
    bevel(d, (x, y, x + 8, y + 8), BTN_LO if pressed else BTN_HI, BTN_LO if pressed else BTN, EDGE)
    c = ACCENT if pressed else GLYPH
    if kind == "options":
        d.rectangle([x + 2, y + 3, x + 6, y + 5], fill=c)
    elif kind == "min":
        d.line([x + 2, y + 6, x + 6, y + 6], fill=c)
    elif kind == "close":
        d.line([x + 2, y + 2, x + 6, y + 6], fill=c)
        d.line([x + 6, y + 2, x + 2, y + 6], fill=c)
    elif kind == "shade":
        d.rectangle([x + 2, y + 2, x + 6, y + 3], fill=c)
    elif kind == "unshade":
        d.rectangle([x + 2, y + 5, x + 6, y + 6], fill=c)


def titlebar_bmp():
    img, d = new(344, 87)
    title_strip(img, d, 0, TITLE_ACTIVE, GLYPH)
    title_strip(img, d, 15, TITLE_INACTIVE, LABEL)
    title_strip(img, d, 57, TITLE_ACTIVE, GLYPH)  # easter egg variants: same look
    title_strip(img, d, 72, TITLE_INACTIVE, LABEL)
    # Windowshade backgrounds (active y=29, inactive y=42).
    for y, colors in ((29, TITLE_ACTIVE), (42, TITLE_INACTIVE)):
        vgradient(img, (27, y, 301, y + 13), *colors)
        d.rectangle([27, y, 301, y + 13], outline=EDGE)
    for x, kind in ((0, "options"), (9, "min"), (18, "close")):
        tb_button(d, x, 0, kind, False)
        tb_button(d, x, 9, kind, True)
    tb_button(d, 0, 18, "shade", False)
    tb_button(d, 9, 18, "shade", True)
    tb_button(d, 0, 27, "unshade", False)
    tb_button(d, 9, 27, "unshade", True)
    # Shade-mode position bar.
    d.rectangle([0, 36, 16, 42], fill=PANEL_LO)
    for x in (17, 20, 23):
        d.rectangle([x, 36, x + 2, 42], fill=BTN_HI)
    # Clutter bar (O A I D V), normal and disabled, plus pressed letters.
    letters = [("o", 3, 8), ("a", 11, 7), ("i", 18, 7), ("d", 25, 8), ("v", 33, 7)]
    for x, color in ((304, LABEL), (312, BTN_LO)):
        d.rectangle([x, 0, x + 7, 42], fill=PANEL_LO)
        d.line([x, 0, x, 42], fill=EDGE)
        for ch, top, _ in letters:
            glyph(d, x + 2, top + 1, ch, color)
    for i, (ch, top, h) in enumerate(letters):
        x, y = 304 + i * 8, 47 + top - 3
        d.rectangle([x, y, x + 7, y + h - 1], fill=EDGE)
        glyph(d, x + 2, y + 1, ch, ACCENT)
    return img


# ---------------------------------------------------------------- cbuttons.bmp
def cbuttons_bmp():
    img, d = new(136, 36)
    specs = [(0, 23, "prev"), (23, 23, "play"), (46, 23, "pause"), (69, 23, "stop"), (92, 22, "next"), (114, 22, "eject")]
    for x, w, kind in specs:
        h = 16 if kind == "eject" else 18
        for pressed, y in ((False, 0), (True, h)):
            bevel(d, (x, y, x + w - 1, y + h - 1), BTN_LO if pressed else BTN_HI, BTN_LO if pressed else BTN, EDGE)
            c = ACCENT if pressed else GLYPH
            cx, cy = x + w // 2, y + h // 2
            if kind == "play":
                d.polygon([(cx - 3, cy - 4), (cx - 3, cy + 4), (cx + 4, cy)], fill=c)
            elif kind == "pause":
                d.rectangle([cx - 4, cy - 4, cx - 2, cy + 4], fill=c)
                d.rectangle([cx + 1, cy - 4, cx + 3, cy + 4], fill=c)
            elif kind == "stop":
                d.rectangle([cx - 4, cy - 4, cx + 3, cy + 3], fill=c)
            elif kind == "prev":
                d.rectangle([cx - 5, cy - 4, cx - 4, cy + 4], fill=c)
                d.polygon([(cx + 3, cy - 4), (cx + 3, cy + 4), (cx - 3, cy)], fill=c)
            elif kind == "next":
                d.polygon([(cx - 4, cy - 4), (cx - 4, cy + 4), (cx + 2, cy)], fill=c)
                d.rectangle([cx + 3, cy - 4, cx + 4, cy + 4], fill=c)
            elif kind == "eject":
                d.polygon([(cx - 5, cy + 1), (cx + 4, cy + 1), (cx, cy - 4)], fill=c)
                d.rectangle([cx - 5, cy + 3, cx + 4, cy + 4], fill=c)
    return img


# ---------------------------------------------------------------- small sheets
def playpaus_bmp():
    img, d = new(42, 9, LCD_BG)
    d.polygon([(2, 1), (2, 7), (7, 4)], fill=LCD_FG)  # playing
    d.rectangle([10, 1, 11, 7], fill=LCD_FG)  # paused
    d.rectangle([14, 1, 15, 7], fill=LCD_FG)
    d.rectangle([20, 2, 24, 6], fill=LCD_FG)  # stopped
    d.rectangle([28, 0, 35, 8], fill=LCD_BG)
    d.rectangle([37, 2, 37, 6], fill=LCD_DIM)  # not working (36..38)
    d.rectangle([40, 2, 40, 6], fill=LCD_FG)  # working (39..41)
    return img


def monoster_bmp():
    img, d = new(58, 24, LCD_BG)
    for y, color in ((0, LCD_FG), (12, LCD_DIM)):
        text(d, 3, y + 3, "stereo", color)
        text(d, 32, y + 3, "mono", color)
    return img


# Seven-segment digits inside 9x13: horizontal bars at x 2..6, y 0/6/12; vertical
# bars at x 1 and 7. Digit 2's middle bar (x 20..24, y 6) doubles as the minus sign.
SEGMENTS = {
    "0": "abcdef", "1": "bc", "2": "abged", "3": "abgcd", "4": "fgbc",
    "5": "afgcd", "6": "afgedc", "7": "abc", "8": "abcdefg", "9": "abcdfg",
}


def seven_seg(d, x, digit, color, dim):
    segs = {
        "a": [(x + 2, 0), (x + 6, 0)], "g": [(x + 2, 6), (x + 6, 6)], "d": [(x + 2, 12), (x + 6, 12)],
        "f": [(x + 1, 1), (x + 1, 5)], "b": [(x + 7, 1), (x + 7, 5)],
        "e": [(x + 1, 7), (x + 1, 11)], "c": [(x + 7, 7), (x + 7, 11)],
    }
    on = SEGMENTS[digit]
    for name, line in segs.items():
        d.line(line, fill=color if name in on else dim)


def numbers_bmp():
    img, d = new(99, 13, LCD_BG)
    for i in range(10):
        seven_seg(d, i * 9, str(i), LCD_FG, (10, 24, 16))
    return img


def text_bmp():
    img, d = new(155, 74, LCD_BG)
    for ch, (row, col) in TEXT_LAYOUT.items():
        glyph(d, col * 5 + 1, row * 6, ch, LCD_FG)
    return img


def slider_sheet(width, frame_x, frame_w, fill_for):
    img, d = new(width, 433, PANEL_LO)
    for i in range(28):
        y = i * 15
        x0 = frame_x
        d.rectangle([x0, y, x0 + frame_w - 1, y + 12], fill=PANEL_LO)
        d.rectangle([x0 + 2, y + 5, x0 + frame_w - 3, y + 7], fill=EDGE)
        fill_for(d, i, x0, y, frame_w)
    for x, pressed in ((0, True), (15, False)):
        bevel(d, (x, 422, x + 13, 432), BTN_HI, BTN_LO if pressed else BTN_HI, EDGE)
        d.rectangle([x + 5, 425, x + 8, 429], fill=ACCENT if pressed else BTN)
    return img


def volume_bmp():
    def fill(d, i, x0, y, w):
        t = i / 27
        length = int((w - 5) * t)
        color = (int(46 + 180 * t), int(232 - 120 * t), 80)
        if length > 0:
            d.rectangle([x0 + 2, y + 5, x0 + 2 + length, y + 7], fill=color)

    return slider_sheet(68, 0, 68, fill)


def balance_bmp():
    def fill(d, i, x0, y, w):
        mid = x0 + w // 2
        length = int((w // 2 - 3) * i / 27)
        color = (int(46 + 180 * i / 27), int(232 - 120 * i / 27), 80)
        d.line([mid, y + 4, mid, y + 8], fill=LABEL)
        if length > 0:
            d.rectangle([mid - length, y + 5, mid + length, y + 7], fill=color)

    return slider_sheet(47, 9, 38, fill)


def posbar_bmp():
    img, d = new(307, 10, PANEL_LO)
    inset(d, (0, 3, 247, 6), EDGE)
    for x, pressed in ((248, False), (278, True)):
        bevel(d, (x, 0, x + 28, 9), BTN_HI, BTN_LO if pressed else BTN, EDGE)
        d.line([x + 14, 2, x + 14, 7], fill=ACCENT if pressed else BTN_LO)
    return img


def shufrep_bmp():
    img, d = new(92, 85, PANEL_LO)

    def toggle(x, y, w, h, label, on, pressed):
        bevel(d, (x, y, x + w - 1, y + h - 1), BTN_LO if pressed else BTN_HI, BTN_LO if pressed else BTN, EDGE)
        d.rectangle([x + 3, y + h // 2 - 1, x + 4, y + h // 2], fill=ACCENT if on else BTN_LO)
        text(d, x + 7, y + (h - 5) // 2, label, GLYPH)

    for i, (on, pressed) in enumerate(((False, False), (False, True), (True, False), (True, True))):
        toggle(0, i * 15, 28, 15, "rep", on, pressed)
        toggle(28, i * 15, 47, 15, "shuffle", on, pressed)
    for x, label in ((0, "eq"), (23, "pl")):
        for dx, dy, on, pressed in ((0, 61, False, False), (0, 73, True, False), (46, 61, False, True), (46, 73, True, True)):
            toggle(x + dx, dy, 23, 12, label, on, pressed)
    return img


# ---------------------------------------------------------------- pledit.bmp
def small_button(img, d, x, y, label, pressed, w=22, h=18):
    bevel(d, (x, y, x + w - 1, y + h - 1), BTN_LO if pressed else BTN_HI, BTN_LO if pressed else BTN, EDGE)
    tw = len(label) * 4 - 1
    text(d, x + (w - tw) // 2, y + (h - 5) // 2, label, ACCENT if pressed else GLYPH)


def pledit_bmp():
    img, d = new(280, 186, PANEL_LO)

    # Top bar, focused (y=0) and unfocused (y=21).
    for y, colors, label in ((0, TITLE_ACTIVE, GLYPH), (21, TITLE_INACTIVE, LABEL)):
        for x, w in ((0, 25), (26, 100), (127, 25), (153, 25)):
            vgradient(img, (x, y, x + w - 1, y + 19), *colors)
            d.line([x, y + 19, x + w - 1, y + 19], fill=EDGE)
        d.line([0, y, 0, y + 19], fill=colors[0])
        d.line([177, y, 177, y + 19], fill=EDGE)
        for gx in range(129, 151, 2):  # grip texture in the tile
            d.line([gx, y + 7, gx, y + 12], fill=PANEL_LO)
        text(d, 26 + (100 - 31) // 2, y + 8, "playlist", label)
        # Close and shade glyphs drawn into the top-right corner.
        tb_button(d, 153 + 14, y + 3, "close", False)
        tb_button(d, 153 + 4, y + 3, "shade", False)

    # Side tiles: left border, right border with scroll track.
    # Flat fills: these tile vertically, so a gradient would band every 29 px.
    d.rectangle([0, 42, 11, 70], fill=PANEL)
    d.line([0, 42, 0, 70], fill=PANEL_HI)
    d.line([11, 42, 11, 70], fill=EDGE)
    d.rectangle([31, 42, 50, 70], fill=PANEL)
    d.line([31, 42, 31, 70], fill=EDGE)
    d.line([50, 42, 50, 70], fill=EDGE)
    d.rectangle([36, 42, 43, 70], fill=PANEL_LO)

    # Pressed close/shade/expand, scroll handle.
    tb_button(d, 52, 42, "close", True)
    tb_button(d, 62, 42, "shade", True)
    tb_button(d, 150, 42, "unshade", True)
    for x, pressed in ((52, False), (61, True)):
        bevel(d, (x, 53, x + 7, 70), BTN_HI, BTN_LO if pressed else BTN, EDGE)
        for gy in (59, 61, 63):
            d.line([x + 2, gy, x + 5, gy], fill=ACCENT if pressed else BTN_LO)

    # Windowshade pieces.
    for y, colors in ((42, TITLE_ACTIVE), (57, TITLE_INACTIVE)):
        vgradient(img, (72, y, 96, y + 13), *colors)
        vgradient(img, (99, y, 148, y + 13), *colors)

    # Bottom tile and visualizer background.
    vgradient(img, (179, 0, 203, 37), PANEL_HI, PANEL_LO)
    vgradient(img, (205, 0, 279, 37), PANEL_HI, PANEL_LO)
    inset(d, (208, 9, 276, 30))

    # Bottom-left: menu buttons.
    vgradient(img, (0, 72, 124, 109), PANEL_HI, PANEL_LO)
    for x, label in ((14, "add"), (43, "rem"), (72, "sel"), (101, "misc")):
        small_button(img, d, x, 80, label, False)

    # Bottom-right: running time, mini transport, mini time, list button, scroll arrows, grip.
    bx = 126
    vgradient(img, (bx, 72, bx + 149, 109), PANEL_HI, PANEL_LO)
    inset(d, (bx + 4, 72 + 7, bx + 98, 72 + 18))
    glyphs = ["prev", "play", "pause", "stop", "next", "eject"]
    widths = [7, 10, 10, 10, 10, 9]
    gx = bx + 3
    for kind, w in zip(glyphs, widths):
        cx, cy = gx + w // 2, 72 + 26
        if kind == "play":
            d.polygon([(cx - 2, cy - 3), (cx - 2, cy + 3), (cx + 2, cy)], fill=GLYPH)
        elif kind == "pause":
            d.rectangle([cx - 3, cy - 3, cx - 2, cy + 3], fill=GLYPH)
            d.rectangle([cx + 1, cy - 3, cx + 2, cy + 3], fill=GLYPH)
        elif kind == "stop":
            d.rectangle([cx - 3, cy - 3, cx + 2, cy + 2], fill=GLYPH)
        elif kind == "prev":
            d.polygon([(cx + 2, cy - 3), (cx + 2, cy + 3), (cx - 2, cy)], fill=GLYPH)
        elif kind == "next":
            d.polygon([(cx - 3, cy - 3), (cx - 3, cy + 3), (cx + 1, cy)], fill=GLYPH)
        elif kind == "eject":
            d.polygon([(cx - 3, cy + 1), (cx + 3, cy + 1), (cx, cy - 3)], fill=GLYPH)
        gx += w
    inset(d, (bx + 63, 72 + 21, bx + 92, 72 + 31))
    small_button(img, d, bx + 106, 80, "list", False)
    for y, up in ((74, True), (80, False)):
        x0 = bx + 135
        pts = [(x0, y + 4), (x0 + 7, y + 4), (x0 + 3, y)] if up else [(x0, y), (x0 + 7, y), (x0 + 3, y + 4)]
        d.polygon(pts, fill=GLYPH)
    for i in range(0, 12, 3):
        d.line([bx + 148 - i, 72 + 36, bx + 148, 72 + 36 - i], fill=LABEL)

    # Pop-up menu items: normal at x, selected at x + 23; bars 3px wide.
    menus = [
        (0, 48, ["url", "dir", "file"]),
        (54, 100, ["all", "crop", "sel", "misc"]),
        (104, 150, ["inv", "none", "all"]),
        (154, 200, ["sort", "info", "opts"]),
        (204, 250, ["new", "save", "load"]),
    ]
    for x, bar, labels in menus:
        for i, label in enumerate(labels):
            y = 111 + i * 19
            small_button(img, d, x, y, label, False)
            small_button(img, d, x + 23, y, label, True)
        d.rectangle([bar, 111, bar + 2, 111 + len(labels) * 18 - 1], fill=ACCENT)
    return img


# ---------------------------------------------------------------- eqmain.bmp / eq_ex.bmp
def eq_slider_frame(d, x, y, level):
    """14x63 slider well; `level` 0..27 fills from the center toward the thumb side."""
    d.rectangle([x, y, x + 13, y + 62], fill=PANEL_LO)
    d.rectangle([x + 5, y + 1, x + 8, y + 61], fill=EDGE)
    mid = y + 31
    t = level / 27
    # Color drifts from cool (cut) to warm (boost).
    color = (int(46 + 190 * t), int(200 - 90 * abs(t - 0.5) * 2 * 0.5), int(220 - 170 * t))
    top = y + 1 + int((1 - t) * 58)
    lo, hi = sorted((mid, top))
    d.rectangle([x + 6, lo, x + 7, hi], fill=color)


def eqmain_bmp():
    img, d = new(275, 315, PANEL_LO)
    # Window body (title bar sprite covers the top 14 rows).
    vgradient(img, (0, 0, 274, 115), PANEL_HI, PANEL_LO)
    d.rectangle([0, 0, 274, 115], outline=EDGE)
    inset(d, (85, 16, 199, 36), LCD_BG)  # graph well
    text(d, 45, 38, "+12db", LABEL)
    text(d, 45, 66, "+0db", LABEL)
    text(d, 45, 97, "-12db", LABEL)
    text(d, 17, 104, "preamp", LABEL)
    for i, label in enumerate(["70", "180", "320", "600", "1k", "3k", "6k", "12k", "14k", "16k"]):
        cx = 78 + i * 18 + 7
        text(d, cx - (len(label) * 4 - 1) // 2, 104, label, LABEL)

    # ON / AUTO buttons: off, on, off pressed, on pressed.
    for (on_x, auto_x, lit, pressed) in ((10, 36, False, False), (69, 95, True, False), (128, 154, False, True), (187, 213, True, True)):
        for x, w, label in ((on_x, 26, "on"), (auto_x, 32, "auto")):
            bevel(d, (x, 119, x + w - 1, 130), BTN_LO if pressed else BTN_HI, BTN_LO if pressed else BTN, EDGE)
            d.rectangle([x + 3, 123, x + 4, 126], fill=ACCENT if lit else BTN_LO)
            text(d, x + 7, 122, label, GLYPH)

    # Close button, normal and pressed.
    tb_button(d, 0, 116, "close", False)
    tb_button(d, 0, 125, "close", True)

    # Title bars: focused y=134, unfocused y=149.
    for y, colors, label in ((134, TITLE_ACTIVE, GLYPH), (149, TITLE_INACTIVE, LABEL)):
        vgradient(img, (0, y, 274, y + 13), *colors)
        d.rectangle([0, y, 274, y + 13], outline=EDGE)
        for gx in range(20, 100, 2):
            d.line([gx, y + 4, gx, y + 9], fill=PANEL_LO if gx % 4 else colors[0])
        for gx in range(176, 242, 2):
            d.line([gx, y + 4, gx, y + 9], fill=PANEL_LO if gx % 4 else colors[0])
        text(d, 112, y + 4, "equalizer", label)
        tb_button(d, 254, y + 3, "shade", False)
        tb_button(d, 264, y + 3, "close", False)

    # Slider wells: 28 frames, 14 per row, 15 px apart, rows 65 px apart.
    for i in range(28):
        eq_slider_frame(d, 13 + (i % 14) * 15, 164 + (i // 14) * 65, i)

    # Slider thumbs.
    for y, pressed in ((164, False), (176, True)):
        bevel(d, (0, y, 10, y + 10), BTN_HI, BTN_LO if pressed else BTN, EDGE)
        d.line([2, y + 5, 8, y + 5], fill=ACCENT if pressed else BTN_LO)

    # Presets button.
    for y, pressed in ((164, False), (176, True)):
        bevel(d, (224, y, 267, y + 11), BTN_LO if pressed else BTN_HI, BTN_LO if pressed else BTN, EDGE)
        text(d, 225 + (44 - 27) // 2, y + 3, "presets", ACCENT if pressed else GLYPH)

    # Graph background, line colors (one per row, warm at the top), preamp line.
    d.rectangle([0, 294, 112, 312], fill=LCD_BG)
    for x in range(0, 113, 4):
        for y in range(296, 312, 4):
            d.point((x, y), fill=(18, 26, 22))
    d.line([0, 303, 112, 303], fill=(24, 40, 30))
    for i in range(19):
        t = i / 18
        d.point((115, 294 + i), fill=(int(240 - 190 * t), int(90 + 140 * t), 70))
    d.line([0, 314, 112, 314], fill=LABEL)
    return img


def eq_ex_bmp():
    img, d = new(275, 82, PANEL_LO)
    for y, colors in ((0, TITLE_ACTIVE), (15, TITLE_INACTIVE)):
        vgradient(img, (0, y, 274, y + 13), *colors)
        d.rectangle([0, y, 274, y + 13], outline=EDGE)
        text(d, 6, y + 4, "eq", GLYPH)
    for x in (1, 4, 7, 11, 14, 17):
        d.rectangle([x, 30, x + 2, 36], fill=BTN_HI)
    tb_button(d, 1, 38, "unshade", True)
    tb_button(d, 1, 47, "shade", True)
    tb_button(d, 11, 38, "close", False)
    tb_button(d, 11, 47, "close", True)
    return img


def placeholder(w, h):
    """Sheets for windows not built yet (equalizer, playlist, generic)."""
    img, d = new(w, h)
    vgradient(img, (0, 0, w - 1, h - 1), PANEL_HI, PANEL_LO)
    return img


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    sheets = {
        "main": main_bmp(), "titlebar": titlebar_bmp(), "cbuttons": cbuttons_bmp(),
        "playpaus": playpaus_bmp(), "monoster": monoster_bmp(), "numbers": numbers_bmp(),
        "text": text_bmp(), "volume": volume_bmp(), "balance": balance_bmp(),
        "posbar": posbar_bmp(), "shufrep": shufrep_bmp(),
        "eqmain": eqmain_bmp(), "eq_ex": eq_ex_bmp(),
        "pledit": pledit_bmp(), "gen": placeholder(194, 109), "genex": placeholder(100, 64),
    }
    for name, img in sheets.items():
        img.save(OUT / f"{name}.png", optimize=True)
    print(f"wrote {len(sheets)} sheets to {OUT}")


if __name__ == "__main__":
    main()
