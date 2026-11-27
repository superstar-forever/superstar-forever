#!/usr/bin/env python3
"""Draw the lobby news banner: the game's title logo with "Offline Edition" below.

Usage: tools/news_banner.py --game <id> [OUT.png]   (preview; the default is work/<game>/news-banner.png)

tools/build-server-data.py calls draw() and puts the result at
server-data/cdn/offline.local/news/offline-edition.png, the image of the Promotion.getNewsData entry in
games/<game>/api-overrides.json. The logo is the title screen's, two textures in the game's Unity
data ("SUPER STAR" and the group's name), read with UnityPy from work/<game>/unpacked/, so the
picture isn't in git. The text is set in Montserrat (tools/fonts/, SIL Open Font License),
close to the group-name lettering. game.json "newsBannerColors": the background gradient.

The lobby slot is about 2.7:1 and stretches the image to fill it; its page dot sits at the bottom
center; the left-aligned logo block keeps clear of it.
"""

import random, sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import ROOT

W, H = 680, 250
LOGO_PARTS = ("title_logo_left", "title_logo_right")  # stacked like on the title screen
TEXT = "Offline Edition"
FONT = ROOT / "tools/fonts/Montserrat.ttf"


def logo_parts(game) -> list[Image.Image]:
    """The title logo textures, found by name in the game's Unity data files."""
    import UnityPy

    data = game.unpacked / "base/assets/bin/Data"
    found = {}
    names = [n.encode() for n in LOGO_PARTS]
    for path in sorted(p for p in data.iterdir() if p.is_file()):
        blob = path.read_bytes()
        if not any(n in blob for n in names):  # loading every file with UnityPy takes minutes
            continue
        for obj in UnityPy.load(str(path)).objects:
            if obj.type.name != "Texture2D":
                continue
            tex = obj.read()
            if tex.m_Name in LOGO_PARTS and tex.m_Name not in found:
                found[tex.m_Name] = tex.image.convert("RGBA")
        if len(found) == len(LOGO_PARTS):
            return [found[n] for n in LOGO_PARTS]
    raise SystemExit(
        f"news banner: textures {', '.join(n for n in LOGO_PARTS if n not in found)} "
        f"not found in {data} (run tools/unpack.sh first)"
    )


def background(left, right) -> Image.Image:
    img = Image.new("RGBA", (W, H))
    d = ImageDraw.Draw(img)
    for x in range(W):
        t = x / (W - 1)
        d.line([(x, 0), (x, H)], fill=tuple(int(a + (b - a) * t) for a, b in zip(left, right)) + (255,))
    rnd = random.Random(3)
    sparkles = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sparkles)
    for _ in range(60):
        x, y, r = rnd.randrange(W), rnd.randrange(H), rnd.choice([1, 1, 2, 3])
        sd.ellipse([x - r, y - r, x + r, y + r], fill=(255, 255, 255, rnd.randrange(60, 170)))
    return Image.alpha_composite(img, sparkles)


def with_shadow(img: Image.Image, layer: Image.Image) -> Image.Image:
    """Composite a white layer over img with a soft dark shadow, so it reads on light colors too."""
    alpha = layer.getchannel("A")
    shadow = Image.new("RGBA", layer.size, (20, 10, 60, 0))
    shadow.putalpha(alpha.point(lambda a: a * 0.55).filter(ImageFilter.GaussianBlur(4)))
    moved = Image.new("RGBA", img.size, (0, 0, 0, 0))
    moved.paste(shadow, (2, 3))
    return Image.alpha_composite(Image.alpha_composite(img, moved), layer)


def draw(game, out: Path):
    left, right = logo_parts(game)
    colors = [tuple(int(c[i : i + 2], 16) for i in (0, 2, 4)) for c in game.cfg["newsBannerColors"]]
    img = background(*colors)

    # Logo block: both parts at one scale, stacked and left-aligned, filling the banner's height
    # (minus a margin for the slot's frame). The page dot at the bottom center stays clear, since
    # the short "Offline Edition" line ends left of it.
    margin = round(H * 0.09)
    gap_r, text_r = 0.18, 0.42  # gaps and text size, relative to the height of the group name
    unit_h = left.height + right.height * (1 + 3 * gap_r) + right.height * text_r * 0.75
    scale = min((H - 2 * margin) / unit_h, (W - 2 * margin) / max(left.width, right.width))
    left = left.resize((round(left.width * scale), round(left.height * scale)), Image.LANCZOS)
    right = right.resize((round(right.width * scale), round(right.height * scale)), Image.LANCZOS)
    gap = round(right.height * gap_r)

    font = ImageFont.truetype(str(FONT), round(right.height * text_r))
    font.set_variation_by_axes([700])  # bold, like the group-name lettering
    box = font.getbbox(TEXT)
    text_h = box[3] - box[1]

    block_h = left.height + gap + right.height + gap * 2 + text_h
    x0 = margin
    y0 = (H - block_h) // 2

    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    layer.alpha_composite(left, (x0, y0))
    layer.alpha_composite(right, (x0, y0 + left.height + gap))
    ty = y0 + left.height + gap + right.height + gap * 2 - box[1]
    ImageDraw.Draw(layer).text((x0 + round(scale * 4), ty), TEXT, font=font, fill=(255, 255, 255, 255))

    img = with_shadow(img, layer)
    out.parent.mkdir(parents=True, exist_ok=True)
    img.convert("RGB").save(out, optimize=True)


if __name__ == "__main__":
    from game import select

    game, argv = select()
    out = Path(argv[1]) if len(argv) > 1 else game.work / "news-banner.png"
    draw(game, out)
    print(out)
