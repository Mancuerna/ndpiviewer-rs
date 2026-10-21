"""Render the original slide-and-cell icon (requires Pillow, development only)."""
from pathlib import Path

from PIL import Image, ImageDraw

SCALE = 4
SIZE = 256
assets = Path(__file__).resolve().parents[1] / "assets"
image = Image.new("RGBA", (SIZE * SCALE, SIZE * SCALE))
draw = ImageDraw.Draw(image)


def box(coords):
    return tuple(round(value * SCALE) for value in coords)


draw.rounded_rectangle(box((8, 8, 248, 248)), radius=52 * SCALE, fill="#152B46")
draw.rounded_rectangle(box((11, 11, 245, 245)), radius=49 * SCALE,
                       outline="#294966", width=3 * SCALE)

slide = Image.new("RGBA", image.size)
pen = ImageDraw.Draw(slide)
pen.rounded_rectangle(box((41, 85, 215, 171)), radius=13 * SCALE, fill="#DDF8FA")
pen.rounded_rectangle(box((48, 92, 83, 164)), radius=7 * SCALE, fill="#32C8CA")
pen.line(box((57, 110, 74, 110)), fill="#167989", width=5 * SCALE)
pen.line(box((57, 125, 74, 125)), fill="#167989", width=5 * SCALE)
pen.ellipse(box((104, 100, 162, 158)), fill="#845ACB")
pen.ellipse(box((119, 109, 148, 138)), fill="#D871CE")
pen.ellipse(box((167, 111, 190, 134)), fill="#40C9C5")
pen.ellipse(box((175, 142, 187, 154)), fill="#845ACB")
image.alpha_composite(slide.rotate(32, resample=Image.Resampling.BICUBIC))
image = image.resize((SIZE, SIZE), Image.Resampling.LANCZOS)
image.save(assets / "app-icon.png")
image.save(assets / "app-icon.ico", sizes=[(s, s) for s in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
