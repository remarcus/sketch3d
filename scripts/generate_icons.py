"""Regenerate platform icons from the supplied transparent PNG (requires Pillow)."""
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
source = Image.open(root / "sketch3d_icon_2048_transparent.png").convert("RGBA")
output = root / "assets" / "icons"
output.mkdir(parents=True, exist_ok=True)
source.resize((256, 256), Image.Resampling.LANCZOS).save(output / "sketch3d.png")
source.save(output / "sketch3d.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
source.resize((1024, 1024), Image.Resampling.LANCZOS).save(output / "sketch3d.icns")
