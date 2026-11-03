"""PHOTOVAULT: branding (new file).

Outline the word "Photovault" in Inter SemiBold as SVG path data, for
render_branding.js.

Usage: python3 -I wordmark.py <font.ttf> <out.json> <dir containing fontTools>
Writes {"d": ..., "width": ..., "height": ..., "minY": ...} in font units
(y already flipped so that the path is in SVG coordinates).
"""
import json
import sys

sys.path.insert(0, sys.argv[3])

from fontTools.pens.svgPathPen import SVGPathPen  # noqa: E402
from fontTools.pens.transformPen import TransformPen  # noqa: E402
from fontTools.pens.boundsPen import BoundsPen  # noqa: E402
from fontTools.ttLib import TTFont  # noqa: E402

TEXT = "Photovault"
font = TTFont(sys.argv[1])
cmap = font.getBestCmap()
gs = font.getGlyphSet()
hmtx = font["hmtx"]
kern = None

pen = SVGPathPen(gs)
bpen = BoundsPen(gs)
x = 0
tracking = -10  # slight negative tracking (font units), like Encore headings
for ch in TEXT:
    name = cmap[ord(ch)]
    t = (1, 0, 0, -1, x, 0)
    gs[name].draw(TransformPen(pen, t))
    gs[name].draw(TransformPen(bpen, t))
    x += hmtx[name][0] + tracking

xmin, ymin, xmax, ymax = bpen.bounds
json.dump(
    {
        "d": pen.getCommands(),
        "xMin": xmin,
        "yMin": ymin,
        "xMax": xmax,
        "yMax": ymax,
    },
    open(sys.argv[2], "w"),
)
print(xmin, ymin, xmax, ymax)
