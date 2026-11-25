"""从 assets/onezip-256.png 生成稀疏包需要的各尺寸图标。

任务栏、开始菜单按 targetsize/scale 限定名挑图；altform-unplated 是不带底板的版本
（任务栏用它，否则会显示成一块纯色方块）。生成后由 build-package.ps1 调 makepri 建索引。
用法：python tools/make-assets.py
"""
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
src = Image.open(root / "assets" / "onezip-256.png").convert("RGBA")
out = root / "packaging" / "sparse" / "Assets"
for old in out.glob("*.png"):
    old.unlink()


def save(size: int, name: str, pad: float = 0.0) -> None:
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    inner = max(1, round(size * (1 - 2 * pad)))
    icon = src.resize((inner, inner), Image.LANCZOS)
    off = (size - inner) // 2
    canvas.alpha_composite(icon, (off, off))
    canvas.save(out / name)


for t in (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256):
    save(t, f"Square44x44Logo.targetsize-{t}.png")
    save(t, f"Square44x44Logo.targetsize-{t}_altform-unplated.png")
    save(t, f"Square44x44Logo.targetsize-{t}_altform-lightunplated.png")
for scale in (100, 125, 150, 200, 400):
    save(round(44 * scale / 100), f"Square44x44Logo.scale-{scale}.png")
    save(round(150 * scale / 100), f"Square150x150Logo.scale-{scale}.png", pad=0.2)
    save(round(50 * scale / 100), f"StoreLogo.scale-{scale}.png")
print(len(list(out.glob("*.png"))), "files")
