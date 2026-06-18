#!/usr/bin/env python3
"""
Gera os assets de branding do AuroraGBA em assets/branding/.

- auroragba_logo.png: a arte inteira (com o lettering "AuroraGBA").
- auroragba_icon.png: o recorte do GBA completado pra quadrado com o preto
  da arte (útil pra previews/redes; sem o lettering).

Os ícones do launcher Android NÃO saem mais daqui — são gerados pelo
Icon Kitchen (https://icon.kitchen) e copiados pra android/app/src/main/res/.

Uso:
  python3 scripts/gen_icon.py <gba_recortado.png> [--logo <arte_inteira.png>]
"""
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
PAD = (0, 0, 0)  # fundo da arte é preto puro — casa com o letterbox


def pad_to_square(src: Image.Image) -> Image.Image:
    """Centraliza a imagem num quadrado, completando com o preto da arte."""
    w, h = src.size
    side = max(w, h)
    canvas = Image.new("RGB", (side, side), PAD)
    canvas.paste(src, ((side - w) // 2, (side - h) // 2))
    return canvas


def save(img, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)
    print("escrito:", path.relative_to(ROOT))


def opt(flag):
    if flag in sys.argv:
        i = sys.argv.index(flag)
        if i + 1 < len(sys.argv):
            return sys.argv[i + 1]
    return None


def main():
    pos = [a for i, a in enumerate(sys.argv[1:], 1)
           if not a.startswith("--") and sys.argv[i - 1] not in ("--logo",)]
    if not pos:
        sys.exit("uso: gen_icon.py <gba_recortado.png> [--logo <arte.png>]")
    src = Image.open(pos[0]).convert("RGB")
    branding = ROOT / "assets" / "branding"

    logo = opt("--logo")
    if logo:
        save(Image.open(logo).convert("RGB"), branding / "auroragba_logo.png")

    save(pad_to_square(src), branding / "auroragba_icon.png")


if __name__ == "__main__":
    main()
