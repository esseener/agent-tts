"""Prepare white, transparent branding from the supplied double-note chip artwork.

Pillow is a development dependency only; all runtime artwork is embedded in Rust.
"""

from io import BytesIO
import hashlib
import json
import os
from pathlib import Path
import struct

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets/source/chip-double-note-original.png"
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]


def white(mask):
    image = Image.new("RGBA", mask.size, (255, 255, 255, 0))
    image.putalpha(mask)
    return image


def fit(mask, size, padding=0):
    reduced = mask.copy()
    reduced.thumbnail((size - 2 * padding, size - 2 * padding), Image.Resampling.LANCZOS)
    canvas = Image.new("L", (size, size), 0)
    canvas.paste(reduced, ((size - reduced.width) // 2, (size - reduced.height) // 2))
    return canvas


def premultiplied_bgra(image):
    raw = bytearray()
    for red, green, blue, opacity in zip(*[iter(image.tobytes())] * 4):
        raw.extend((blue * opacity // 255, green * opacity // 255,
                    red * opacity // 255, opacity))
    return raw


def wordmark(size, font_path, font_size):
    mask = Image.new("L", size, 0)
    draw = ImageDraw.Draw(mask)
    font = ImageFont.truetype(str(font_path), font_size)
    bounds = draw.textbbox((0, 0), "Agent Voice", font=font)
    width, height = bounds[2] - bounds[0], bounds[3] - bounds[1]
    draw.text(((size[0] - width) // 2 - bounds[0],
               (size[1] - height) // 2 - bounds[1]),
              "Agent Voice", font=font, fill=255)
    return white(mask)


def prepare_assets():
    assets = ROOT / "assets"
    previews = ROOT / "docs/previews"
    previews.mkdir(parents=True, exist_ok=True)
    image = Image.open(SOURCE).convert("RGB")
    if image.size != (2048, 2048):
        raise ValueError("The supplied double-note artwork must remain at its original 2048px size")

    channels = iter(image.tobytes())
    alpha = Image.new("L", image.size)
    # Separate green from the dark background and its glow. All visible output
    # pixels have white RGB; only their opacity carries the antialiased edges.
    alpha.putdata([
        round(max(0, min(255, (green - max(red, blue) - 24) * 255 / 130)))
        for red, green, blue in zip(channels, channels, channels)
    ])
    bounds = alpha.point(lambda value: 255 if value > 32 else 0).getbbox()
    if bounds is None:
        raise ValueError("Logo artwork has no visible content")
    full = alpha.crop(bounds)
    master = fit(full, 1024, 48)
    white(master).save(assets / "logo-white.png")
    white(fit(full, 512, 24)).save(assets / "logo.png")

    # Keep the source chip, double note and seven pins on each side. Exclude
    # external traces and binary fragments without repainting the artwork.
    selection = Image.new("L", image.size, 0)
    draw = ImageDraw.Draw(selection)
    draw.rounded_rectangle((604, 593, 1448, 1401), radius=150, fill=255)
    for box in [(765, 506, 1285, 610),
                (765, 1384, 1285, 1485), (539, 755, 629, 1225),
                (1423, 755, 1514, 1225)]:
        draw.rectangle(box, fill=255)
    compact = ImageChops.darker(alpha, selection).crop((533, 500, 1521, 1492))
    white(fit(compact, 512, 20)).save(assets / "logo-compact-white.png")

    # At 16–24px the concentric borders/pins merge. Use the SAME source's
    # inner rounded chip frame and double note with a little more stroke weight.
    selection = Image.new("L", image.size, 0)
    ImageDraw.Draw(selection).rounded_rectangle((648, 630, 1406, 1362), radius=100, fill=255)
    core = ImageChops.darker(alpha, selection).crop((644, 626, 1410, 1366))

    for size in [64, 128, 256]:
        mask = fit(full, size, max(2, round(size * 48 / 1024)))
        peak = mask.getextrema()[1]
        mask = mask.point(lambda value: round(255 * (value / peak) ** .8))
        frame = white(mask)
        frame.save(assets / f"logo-{size}.png")
        if size in (64, 256):
            (assets / f"logo-{size}.bgra").write_bytes(premultiplied_bgra(frame))

    font_path = Path(os.environ.get("WINDIR", "C:/Windows")) / "Fonts/seguisb.ttf"
    horizontal = Image.new("RGBA", (1600, 400), (255, 255, 255, 0))
    horizontal.alpha_composite(white(master.resize((368, 368), Image.Resampling.LANCZOS)), (16, 16))
    horizontal.alpha_composite(wordmark((1168, 400), font_path, 200), (408, 0))
    horizontal.save(assets / "logo-horizontal-white.png")
    stacked = Image.new("RGBA", (1024, 1152), (255, 255, 255, 0))
    stacked.alpha_composite(white(master.resize((896, 896), Image.Resampling.LANCZOS)), (64, 16))
    stacked.alpha_composite(wordmark((1024, 224), font_path, 152), (0, 912))
    stacked.save(assets / "logo-stacked-white.png")
    wordmark((1280, 256), font_path, 208).save(assets / "wordmark-white.png")

    def icon_frame(size):
        source = core if size <= 24 else compact if size <= 64 else full
        if size <= 48:
            source = source.filter(ImageFilter.MaxFilter(3))
        mask = fit(source, size, max(1, round(size * .035)))
        if size <= 64:
            mask = mask.point(lambda value: 0 if value < 16 else
                              round(255 * min(1, (value - 16) / 112) ** .72))
        return white(mask)

    frames = [icon_frame(size) for size in SIZES]
    # Write prepared frames directly, retaining their individual size treatment.
    payloads = []
    for frame in frames:
        buffer = BytesIO()
        frame.save(buffer, format="PNG")
        payloads.append(buffer.getvalue())
    offset = 6 + 16 * len(SIZES)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(SIZES)))
    for size, payload in zip(SIZES, payloads):
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0,
                                     1, 32, len(payload), offset))
        offset += len(payload)
    (assets / "AgentVoice.ico").write_bytes(bytes(directory) + b"".join(payloads))

    preview = Image.new("RGB", (960, 300), (10, 10, 10))
    draw = ImageDraw.Draw(preview)
    draw.text((20, 16), "WHITE DOUBLE-NOTE CHIP / WINDOWS ICONS", fill="white")
    for index, (size, frame) in enumerate(zip(SIZES[:7], frames[:7])):
        x = 20 + index * 132
        preview.paste(frame, (x, 58), frame)
        enlarged = frame.resize((size * 2, size * 2), Image.Resampling.NEAREST)
        preview.paste(enlarged, (x, 158), enlarged)
        draw.text((x, 132), f"{size} px", fill="white")
    preview.save(previews / "icon-sharp.png")

    board = Image.new("RGB", (1600, 1120), (5, 5, 5))
    draw = ImageDraw.Draw(board)
    draw.text((48, 32), "AGENT VOICE / WHITE LOGO FAMILY", fill="white")
    for name, size, position, label in [
        ("logo-white.png", (460, 460), (32, 100), "FULL ARTWORK"),
        ("logo-compact-white.png", (380, 380), (566, 140), "COMPACT CHIP"),
        ("logo-stacked-white.png", (400, 450), (1110, 100), "STACKED"),
        ("logo-horizontal-white.png", (1456, 364), (72, 680), "HORIZONTAL"),
    ]:
        frame = Image.open(assets / name).convert("RGBA")
        frame.thumbnail(size, Image.Resampling.LANCZOS)
        board.paste(frame, position, frame)
        draw.text((position[0] + 16, position[1] - 32), label, fill=(170, 170, 170))
    board.save(previews / "logo-family.png")
    dark = Image.new("RGB", (512, 512), (5, 5, 5))
    logo = Image.open(assets / "logo.png")
    dark.paste(logo, (0, 0), logo)
    dark.save(previews / "logo-on-dark.png")

    manifest = {
        "source": SOURCE.relative_to(ROOT).as_posix(),
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "color": "#FFFFFF", "background": "transparent", "icon_sizes": SIZES,
        "forms": {name: list(Image.open(assets / name).size) for name in [
            "logo-white.png", "logo.png", "logo-compact-white.png",
            "logo-horizontal-white.png", "logo-stacked-white.png", "wordmark-white.png"]},
    }
    (assets / "branding.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print("Prepared all-white full, compact, horizontal, stacked and wordmark logos; 10 Windows icon sizes.")


if __name__ == "__main__":
    prepare_assets()
