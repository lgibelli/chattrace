#!/usr/bin/env python3
"""Generate assets/logo.svg (app icon) and assets/mark.svg (icon without the
background tile, for use on dark UI). A chat bubble holding fingerprint ridges:
"chat" + "trace". Render the PNG/ICO/ICNS files with scripts/make_icons.sh.
SPDX-License-Identifier: GPL-3.0-or-later
"""
import math, os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NAVY_TOP, NAVY_BOTTOM = "#16305a", "#08142a"
TEAL_TOP, TEAL_BOTTOM = "#3be0cf", "#13a4a0"
RIDGE = "#0a1a33"

CX, CY = 512, 468  # fingerprint centre (inside the bubble)

def arc(r, a1, a2, rx_scale=1.0):
    """Open arc from angle a1 to a2 (degrees, 0 = right, clockwise in SVG coords)."""
    p = lambda a: (CX + r * rx_scale * math.cos(math.radians(a)), CY + r * math.sin(math.radians(a)))
    (x1, y1), (x2, y2) = p(a1), p(a2)
    large = 1 if (a2 - a1) % 360 > 180 else 0
    return f'M{x1:.1f} {y1:.1f} A{r*rx_scale:.1f} {r:.1f} 0 {large} 1 {x2:.1f} {y2:.1f}'

# Ridges: concentric, open towards the lower left like a real whorl; a few gaps.
ridges = [
    arc(38, 200, 520),
    arc(82, 160, 300), arc(82, 318, 470),
    arc(126, 150, 250), arc(126, 268, 420),
    arc(170, 165, 290), arc(170, 306, 385),
]
ridge_paths = "\n    ".join(f'<path d="{d}"/>' for d in ridges)

bubble = """<path d="M376 238 H648 A150 150 0 0 1 798 388 V552 A150 150 0 0 1 648 702 H462 C420 742 360 782 300 806 C290 810 284 802 290 794 C316 760 330 734 336 697 A150 150 0 0 1 226 552 V388 A150 150 0 0 1 376 238 Z"/>"""

defs = f"""<defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{NAVY_TOP}"/><stop offset="1" stop-color="{NAVY_BOTTOM}"/>
    </linearGradient>
    <linearGradient id="bubble" gradientUnits="userSpaceOnUse" x1="420" y1="238" x2="600" y2="806">
      <stop offset="0" stop-color="{TEAL_TOP}"/><stop offset="1" stop-color="{TEAL_BOTTOM}"/>
    </linearGradient>
    <linearGradient id="rim" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.22"/><stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
  </defs>"""

content = f"""<g fill="url(#bubble)">
    {bubble}
  </g>
  <g fill="none" stroke="{RIDGE}" stroke-width="30" stroke-linecap="round">
    {ridge_paths}
  </g>"""

logo = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <!-- ChatTrace Forensic app icon. SPDX-License-Identifier: GPL-3.0-or-later -->
  {defs}
  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#bg)"/>
  <rect x="101.5" y="101.5" width="821" height="821" rx="184" fill="none" stroke="url(#rim)" stroke-width="3"/>
  {content}
</svg>
"""

mark = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="200 210 624 624" width="624" height="624">
  <!-- ChatTrace Forensic mark. SPDX-License-Identifier: GPL-3.0-or-later -->
  {defs}
  {content}
</svg>
"""

# Simplified variant for 16-48 px: fewer, thicker ridges.
small_ridges = "\n    ".join(f'<path d="{d}"/>' for d in [arc(52, 200, 520), arc(140, 150, 300), arc(140, 322, 430)])
small = logo.replace(ridge_paths, small_ridges).replace('stroke-width="30"', 'stroke-width="58"')

for name, s in (("logo.svg", logo), ("mark.svg", mark), ("logo-small.svg", small)):
    with open(os.path.join(ROOT, "assets", name), "w") as f:
        f.write(s)
print("wrote assets/logo.svg, assets/mark.svg, assets/logo-small.svg")
