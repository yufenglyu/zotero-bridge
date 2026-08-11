"""Generate Zotero Bridge app icons (PNG 32/128/256 + ICO) into src-tauri/icons/."""
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(__file__).parent
SIZES = [32, 128, 256]


def draw_icon(size: int) -> Image.Image:
    scale = 4  # supersample for smooth edges
    s = size * scale
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Rounded square with a blue -> purple diagonal gradient.
    radius = int(s * 0.22)
    grad = Image.new("RGBA", (s, s))
    gd = ImageDraw.Draw(grad)
    c1 = (79, 140, 255)   # #4f8cff
    c2 = (123, 92, 255)   # #7b5cff
    for y in range(s):
        t = y / s
        r = int(c1[0] + (c2[0] - c1[0]) * t)
        g = int(c1[1] + (c2[1] - c1[1]) * t)
        b = int(c1[2] + (c2[2] - c1[2]) * t)
        gd.line([(0, y), (s, y)], fill=(r, g, b, 255))
    mask = Image.new("L", (s, s), 0)
    md = ImageDraw.Draw(mask)
    md.rounded_rectangle([0, 0, s - 1, s - 1], radius=radius, fill=255)
    img.paste(grad, (0, 0), mask)

    # White "Z" stroke.
    d = ImageDraw.Draw(img)
    pad = s * 0.24
    w = max(2, int(s * 0.09))
    top = pad
    bottom = s - pad
    left = pad
    right = s - pad
    d.line([(left, top), (right, top)], fill=(255, 255, 255, 255), width=w)
    d.line([(right, top), (left, bottom)], fill=(255, 255, 255, 255), width=w)
    d.line([(left, bottom), (right, bottom)], fill=(255, 255, 255, 255), width=w)
    # Round the stroke caps.
    for (x, y) in [(left, top), (right, top), (left, bottom), (right, bottom)]:
        d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=(255, 255, 255, 255))

    return img.resize((size, size), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for size in SIZES:
        draw_icon(size).save(OUT / f"{size}x{size}.png")
    draw_icon(256).save(
        OUT / "icon.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )
    print("icons written to", OUT)


if __name__ == "__main__":
    main()
