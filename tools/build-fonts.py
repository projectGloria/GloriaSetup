"""Rebuild assets/fonts from the OFL originals in github.com/google/fonts.

Each variable font is pinned to the static weights the UI uses and subset to Latin + Latin-1 + Latin Extended-A
(Turkish ğ ş ı İ) plus the few symbols the screens draw. Needs: pip install fonttools brotli.
Usage: python tools/build-fonts.py
"""
import io
import pathlib
import os
import urllib.request

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "fonts"
# google/fonts moves on; set GLORIA_FONTS_REF to a commit SHA to rebuild exactly the fonts a release shipped with.
REF = os.environ.get("GLORIA_FONTS_REF", "main")
BASE = f"https://raw.githubusercontent.com/google/fonts/{REF}/ofl/"

FONTS = {
    # family dir, variable file, output prefix, weights, extra fixed axes
    "archivo": ("Archivo[wdth,wght].ttf", "Archivo", [600, 700, 800], {"wdth": 100}),
    "inter": ("Inter[opsz,wght].ttf", "Inter", [400, 500, 600, 700], {"opsz": 14}),
    "jetbrainsmono": ("JetBrainsMono[wght].ttf", "JetBrainsMono", [400, 500, 600, 700], {}),
}

# Basic Latin, Latin-1, Latin Extended-A, general punctuation, arrows, a few symbols used by the UI.
UNICODES = (
    list(range(0x20, 0x7F)) + list(range(0xA0, 0x180)) + list(range(0x2010, 0x2027)) +
    [0x2030, 0x2039, 0x203A, 0x20AC, 0x2122, 0x2190, 0x2191, 0x2192, 0x2193, 0x2026, 0x00B7, 0x2212, 0x00D7]
)


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "GloriaSetup-font-build"})
    with urllib.request.urlopen(req) as r:
        return r.read()


WEIGHT_NAMES = {400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold"}


def family_name(font: TTFont) -> str:
    name = font["name"]
    return str(name.getDebugName(16) or name.getDebugName(1))


def set_names(font: TTFont, family: str, weight: int) -> None:
    """DirectWrite groups by typographic family (16) and picks by usWeightClass, so set both explicitly."""
    style = WEIGHT_NAMES[weight]
    font["OS/2"].usWeightClass = weight
    name = font["name"]
    ps = f"{family.replace(' ', '')}-{style}"
    for nid, val in {1: f"{family} {style}" if weight not in (400, 700) else family,
                     2: "Bold" if weight == 700 else "Regular",
                     4: f"{family} {style}", 6: ps, 16: family, 17: style}.items():
        name.setName(val, nid, 3, 1, 0x409)
    # Drop the Mac-platform records so only the names set above remain.
    name.names = [n for n in name.names if n.platformID == 3]


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for family, (var_file, prefix, weights, fixed) in FONTS.items():
        raw = fetch(BASE + family + "/" + urllib.request.quote(var_file))
        (OUT / f"OFL-{prefix}.txt").write_bytes(fetch(BASE + family + "/OFL.txt"))
        for w in weights:
            font = TTFont(io.BytesIO(raw))
            axes = {"wght": w, **fixed}
            axes = {k: v for k, v in axes.items() if k in [a.axisTag for a in font["fvar"].axes]}
            static = instancer.instantiateVariableFont(font, axes)
            set_names(static, family_name(static), w)
            opts = subset.Options()
            opts.layout_features = ["kern", "liga", "calt", "tnum", "case", "ccmp", "locl", "mark", "mkmk"]
            opts.name_IDs = ["*"]
            opts.notdef_outline = True
            opts.hinting = True
            sub = subset.Subsetter(opts)
            sub.populate(unicodes=UNICODES)
            sub.subset(static)
            path = OUT / f"{prefix}-{w}.ttf"
            static.save(path)
            print(f"{path.name}: {path.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
