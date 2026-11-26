#!/usr/bin/env python3
"""Draw the Finder window background of Ábaco's DMG as an SVG: background.svg.

    pip install fonttools
    NODALIA_DESIGN=/path/to/nodalia-design python3 packaging/macos/dmg/draw_background_abaco.py

It uses nodalia-design's tools/dmg-background/draw_background.py (the layout, islands, type and
rules every Nodalia DMG shares) and adds Ábaco's own picture and arrow to its table of apps, the
way each app already has one. Until the same entry lands in nodalia-design, this file carries it;
once it does, run nodalia-design's script with `abaco` instead and delete this one.

The accent is Ábaco's #A23B72; the darker shade is the same ratio the other apps use.
"""
import importlib.util
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESIGN = Path(os.environ.get("NODALIA_DESIGN", HERE.parents[3] / "nodalia-design"))
SCRIPT = DESIGN / "tools" / "dmg-background" / "draw_background.py"
if not SCRIPT.is_file():
    sys.exit(f"error: {SCRIPT} not found; set NODALIA_DESIGN to a nodalia-design checkout")
spec = importlib.util.spec_from_file_location("draw_background", SCRIPT)
db = importlib.util.module_from_spec(spec)
spec.loader.exec_module(db)

W_ART, H_ART = db.AW, db.AH


def m_abaco(c, d):
    """A counting frame: rods with beads pushed to either side, and the sum it adds up to."""
    o = [f'<rect width="{W_ART}" height="{H_ART}" fill="{c}"/>']
    left, top, right, bottom = 28, 52, W_ART - 28, 240
    o.append(f'<rect x="{left}" y="{top}" width="{right - left}" height="{bottom - top}" rx="8" '
             f'fill="{d}" stroke="#FFFFFF" stroke-width="7"/>')
    rods = 6
    gap = (bottom - top) / (rods + 1)
    counts = (2, 4, 1, 3, 5, 2)  # beads on the left of each rod, of five
    for i in range(rods):
        y = top + gap * (i + 1)
        o.append(f'<line x1="{left}" y1="{y:.1f}" x2="{right}" y2="{y:.1f}" stroke="#FFFFFF" stroke-width="1.6"/>')
        for b in range(5):
            if b < counts[i]:
                x = left + 16 + b * 15
            else:
                x = right - 16 - (4 - b) * 15
            o.append(f'<ellipse cx="{x}" cy="{y:.1f}" rx="7" ry="9" fill="#FFFFFF"/>')
    # the sum, as a result line under the frame
    o.append(f'<rect x="{left}" y="276" width="{right - left}" height="2" fill="#FFFFFF" opacity="0.55"/>')
    o.append(db.text("10 kg · 9.81 m/s²", left, 308, 15, 600, "#FFFFFF"))
    o.append(db.text("= 98.1 N", left, 336, 22, 800, "#FFFFFF"))
    return "".join(o)


def a_abaco(c):
    """A bead slid along a rod toward the install target."""
    ay, al, ar = db.AYc, db.AL, db.AR
    return (f'<line x1="{al}" y1="{ay}" x2="{ar - 12}" y2="{ay}" stroke="{db.INK}" stroke-width="1.6" stroke-linecap="round"/>'
            f'<ellipse cx="{al + 8}" cy="{ay}" rx="6" ry="8" fill="{c}"/>'
            f'<ellipse cx="{al + 22}" cy="{ay}" rx="6" ry="8" fill="{c}"/>'
            + db.arrowhead(ar, ay, db.INK))


db.APPS["abaco"] = ("Ábaco", "Matemáticas con unidades", "#A23B72", "#792C56", m_abaco, a_abaco)

if __name__ == "__main__":
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "background.svg"
    # The shared comment says the drawing lives in nodalia-design; this one's motif lives here.
    comment = "Drawn by tools/dmg-background/draw_background.py in nodalia-design: change it there and regenerate."
    own = "Drawn by packaging/macos/dmg/draw_background_abaco.py on nodalia-design's tools/dmg-background layout: change it there and regenerate."
    out.write_text(db.svg("abaco").replace(comment, own))
    print(f"wrote {out}")
