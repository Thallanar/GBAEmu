#!/usr/bin/env python3
"""
Gera o ícone do app desktop a partir do MESMO ícone do launcher Android.

Junta as camadas do ícone adaptativo (background + foreground, xxxhdpi 432 px),
recorta a zona visível (72/108 do centro, como o Android faz) e aplica a máscara
redonda — o mesmo visual do launcher. Saídas em crates/desktop/assets/:

- auroragba.png: 256×256, embutido no binário como ícone da janela (Linux/Windows)
  e instalado como ícone do .desktop no Linux.
- auroragba.ico: 16–256 px, embutido no .exe do Windows (ícone no Explorer).

Uso:
  python3 scripts/gen_desktop_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
RES = ROOT / "android/app/src/main/res/mipmap-xxxhdpi"
OUT = ROOT / "crates/desktop/assets"
LAYER = 432  # lado das camadas xxxhdpi (108 dp × 4)
SS = 4  # supersampling da máscara (borda do círculo suave)


def main():
    bg = Image.open(RES / "ic_launcher_background.png").convert("RGBA")
    fg = Image.open(RES / "ic_launcher_foreground.png").convert("RGBA")
    full = Image.alpha_composite(bg, fg)

    # Zona visível do adaptive icon: 72 dp dos 108 (18 dp de margem de cada lado).
    m = LAYER * 18 // 108
    icon = full.crop((m, m, LAYER - m, LAYER - m)).resize((256, 256), Image.LANCZOS)

    mask = Image.new("L", (256 * SS, 256 * SS), 0)
    ImageDraw.Draw(mask).ellipse((0, 0, 256 * SS - 1, 256 * SS - 1), fill=255)
    icon.putalpha(mask.resize((256, 256), Image.LANCZOS))

    OUT.mkdir(parents=True, exist_ok=True)
    icon.save(OUT / "auroragba.png")
    icon.save(
        OUT / "auroragba.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("escrito:", OUT / "auroragba.png", "e", OUT / "auroragba.ico")


if __name__ == "__main__":
    main()
