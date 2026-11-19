"""Génère le logo de l'Explorateur (PNG + ICO multi-tailles) dans la DA de l'app.

Un dossier crème qui remplit toute l'icône, sans cadre ni contour, onglet caramel. Rangées bleues = la liste de fichiers,
point orange = un changement en direct.
Usage : py -3 assets/make_icon.py
"""
from pathlib import Path
from PIL import Image, ImageDraw

NAVY = (19, 34, 58, 255)      # #13223A
CREAM = (242, 230, 204, 255)  # crème un peu plus soutenue que #F5EDDC
TAB = (201, 169, 116, 255)    # onglet caramel, contraste avec la crème
BLUE = (46, 94, 158, 255)     # #2E5E9E
LIVE = (242, 140, 50, 255)    # orange « live », plus vif pour les petites tailles

S = 1024
SS = 4  # suréchantillonnage pour des bords nets


def draw(size_px: int) -> Image.Image:
    small = size_px <= 24
    k = SS * size_px / S
    W = int(size_px * SS)
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = lambda v: int(round(v * k))
    # Onglet (plus foncé, il dessine la silhouette du dossier)
    d.rounded_rectangle([s(24), s(96), s(470), s(320)], radius=s(70), fill=TAB)
    # Corps du dossier : presque toute l'icône, sans cadre ni contour
    d.rounded_rectangle([s(24), s(196), s(1000), s(928)], radius=s(110), fill=CREAM)

    # Rangées de fichiers (moins nombreuses et plus épaisses en petit)
    rows = ((420, 640), (620, 780)) if small else ((400, 620), (540, 780), (680, 540))
    h = 56 if small else 40
    for y, x2 in rows:
        d.rounded_rectangle([s(150), s(y - h), s(x2), s(y + h)], radius=s(h), fill=BLUE)
    # Point « live »
    r = 92 if small else 74
    cx, cy = 800, 400 if not small else 420
    d.ellipse([s(cx - r), s(cy - r), s(cx + r), s(cy + r)], fill=LIVE)

    return img.resize((size_px, size_px), Image.LANCZOS)


def main() -> None:
    out = Path(__file__).resolve().parent
    draw(1024).save(out / "icon.png")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [draw(n) for n in sizes]
    frames[-1].save(out / "icon.ico", format="ICO", sizes=[(n, n) for n in sizes], append_images=frames[:-1])
    draw(64).save(out / "icon64.png")
    # Planche de contrôle : tailles réelles sur fond clair et sombre
    sheet = Image.new("RGBA", (sum(sizes) + 20 * len(sizes) + 20, 2 * 256 + 60), (0, 0, 0, 0))
    x = 20
    for n, f in zip(sizes, frames):
        for row, bg in enumerate(((243, 243, 243, 255), (32, 32, 32, 255))):
            tile = Image.new("RGBA", (n + 12, n + 12), bg)
            tile.alpha_composite(f, (6, 6))
            sheet.alpha_composite(tile, (x - 6, 20 + row * (256 + 20)))
        x += n + 20
    sheet.save(out / "icon_sheet.png")
    print("ok:", out / "icon.ico")


if __name__ == "__main__":
    main()
