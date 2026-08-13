"""Build the checked-in Windows icon assets from the supplied square artwork."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageEnhance, ImageFilter


ICON_SIZES = (16, 24, 32, 48, 64, 128, 256)


def build_icon(source_path: Path, output_dir: Path) -> None:
    source = Image.open(source_path).convert("RGB")
    side = min(source.size)
    left = (source.width - side) // 2
    top = (source.height - side) // 2
    square = source.crop((left, top, left + side, top + side))

    # Remove only the near-white outer background. A soft threshold keeps the
    # glowing cyan edge while giving Windows a clean transparent silhouette.
    rgba = square.convert("RGBA")
    pixels = []
    for red, green, blue, _alpha in rgba.get_flattened_data():
        minimum = min(red, green, blue)
        maximum = max(red, green, blue)
        whiteness = minimum - (maximum - minimum) * 0.35
        alpha = max(0, min(255, round((250 - whiteness) * 18)))
        pixels.append((red, green, blue, alpha))
    rgba.putdata(pixels)

    alpha = rgba.getchannel("A").filter(ImageFilter.GaussianBlur(0.6))
    rgba.putalpha(alpha)
    bbox = alpha.getbbox()
    if bbox is None:
        raise ValueError("source image became fully transparent")

    artwork = rgba.crop(bbox)
    canvas_size = 1024
    target_size = 940
    artwork.thumbnail((target_size, target_size), Image.Resampling.LANCZOS)
    canvas = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    position = (
        (canvas_size - artwork.width) // 2,
        (canvas_size - artwork.height) // 2,
    )
    canvas.alpha_composite(artwork, position)

    output_dir.mkdir(parents=True, exist_ok=True)
    png_path = output_dir / "MonitorDDC.png"
    ico_path = output_dir / "MonitorDDC.ico"
    window_icon = canvas.resize((256, 256), Image.Resampling.LANCZOS)
    window_icon.save(png_path, optimize=True)

    # Give small Explorer/taskbar variants slightly more edge contrast.
    icon_base = ImageEnhance.Contrast(canvas).enhance(1.04)
    icon_base.save(ico_path, sizes=[(size, size) for size in ICON_SIZES])

    print(f"PNG: {png_path} ({window_icon.width}x{window_icon.height})")
    print(f"ICO: {ico_path} ({', '.join(map(str, ICON_SIZES))} px)")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("--output-dir", type=Path, default=Path("assets"))
    args = parser.parse_args()
    build_icon(args.source, args.output_dir)


if __name__ == "__main__":
    main()
