#!/usr/bin/env python3
"""Port the NEONWIRE dossier into the naderlabs.io Next app as a real route.

The Artifact build (build_dossier.py) inlines everything as data URIs because
the Artifact CSP blocks external assets. The website has no such constraint, so
here we do the opposite: emit real files under public/neonwire/ and a scoped
stylesheet, so the page loads fast instead of shipping ~1.1 MB of base64.

Outputs (all generated — edit dossier.template.html, not these):
  <web>/public/neonwire/*                       fonts, screenshots, videos
  <web>/app/projects/neonwire/dossier.generated.css
  <web>/app/projects/neonwire/dossier.generated.html

The CSS is scoped under `.nw-dossier` so the dossier's committed dark neon-CRT
world cannot leak into the rest of the site (which is themed).

Usage:  python3 build_web_dossier.py [path/to/naderlabsio]
"""
from __future__ import annotations

import base64
import re
import shutil
import sys
from pathlib import Path

from build_dossier import FONT_DIR, IMAGES, VIDEOS, subset_woff

HERE = Path(__file__).resolve().parent
WEB = Path(sys.argv[1] if len(sys.argv) > 1 else Path.home() / "Code/naderlabsio") / "apps/web"
PUBLIC = WEB / "public" / "neonwire"
ROUTE = WEB / "app" / "projects" / "neonwire"

SCOPE = ".nw-dossier"
# Generic names that would collide with site-wide CSS if left unprefixed.
ANIMS = ["sweep", "pulse", "boot", "eq"]


def scope_css(css: str) -> str:
    """Prefix every selector with SCOPE, rewriting the document-level rules.

    Walks top-level blocks by brace matching so @media bodies recurse and
    @font-face / @keyframes pass through untouched.
    """
    # Strip comments first: they otherwise get swallowed into the *next*
    # rule's selector, and a comment containing a brace would derail the
    # brace-matching walk below.
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    out, i, n = [], 0, len(css)
    while i < n:
        brace = css.find("{", i)
        if brace == -1:
            out.append(css[i:])
            break
        selector = css[i:brace].strip()
        depth, j = 1, brace + 1
        while j < n and depth:
            if css[j] == "{":
                depth += 1
            elif css[j] == "}":
                depth -= 1
            j += 1
        body = css[brace + 1 : j - 1]

        if selector.startswith("@media") or selector.startswith("@supports"):
            out.append(f"{selector}{{{scope_css(body)}}}")
        elif selector.startswith("@keyframes"):
            out.append(f"{selector}{{{body}}}")
        elif selector.startswith("@"):
            out.append(f"{selector}{{{body}}}")
        else:
            parts = []
            for sel in selector.split(","):
                sel = sel.strip()
                if not sel:
                    continue
                if sel in (":root", "body"):
                    parts.append(SCOPE)          # page vars + ground live on the wrapper
                elif sel == "html":
                    parts.append(SCOPE)
                elif sel == "*":
                    parts.append(f"{SCOPE} *")
                else:
                    parts.append(f"{SCOPE} {sel}")
            out.append(f"{', '.join(parts)}{{{body}}}")
        i = j
    return "\n".join(out)


def main() -> None:
    src = (HERE / "dossier.template.html").read_text()

    style = re.search(r"<style>(.*?)</style>", src, re.S).group(1)
    # Everything after </style>, minus the trailing reveal script (reimplemented
    # as a client component so React owns the lifecycle).
    rest = src.split("</style>", 1)[1]
    rest = re.sub(r"<script>.*?</script>", "", rest, flags=re.S).strip()
    rest = re.sub(r"^<title>.*?</title>", "", rest, flags=re.S).strip()

    PUBLIC.mkdir(parents=True, exist_ok=True)
    ROUTE.mkdir(parents=True, exist_ok=True)

    # Fonts — subset exactly as the artifact does, but as real .woff files.
    for token, src_name, out_name in (
        ("%%FONT_REG%%", "JetBrainsMonoNerdFontMono-Regular.ttf", "jbmono-400.woff"),
        ("%%FONT_BOLD%%", "JetBrainsMonoNerdFontMono-Bold.ttf", "jbmono-700.woff"),
    ):
        data_uri, _ = subset_woff(FONT_DIR / src_name)
        raw = base64.b64decode(data_uri.split(",", 1)[1])
        (PUBLIC / out_name).write_bytes(raw)
        style = style.replace(token, f"/neonwire/{out_name}")
        print(f"font  {out_name:24} {len(raw)//1024}KB")

    # Screenshots + videos as real files.
    for token, path in {**IMAGES, **VIDEOS}.items():
        shutil.copy2(path, PUBLIC / path.name)
        rest = rest.replace(token, f"/neonwire/{path.name}")
        print(f"asset {path.name:24} {path.stat().st_size//1024}KB")

    css = scope_css(style)
    # Rename keyframes ONLY at their definition and inside animation
    # declarations. A blanket \bname\b rewrite also hits class selectors —
    # `eq` is both an animation and the .eq equalizer class, and renaming the
    # selector silently unstyles it because the HTML still says class="eq".
    for name in ANIMS:
        css = re.sub(rf"@keyframes\s+{name}\b", f"@keyframes nw-{name}", css)
        css = re.sub(
            rf"(animation(?:-name)?\s*:[^;}}]*?)\b{name}\b",
            rf"\1nw-{name}",
            css,
        )

    (ROUTE / "dossier.generated.css").write_text(
        "/* GENERATED by experiments/fbui/build_web_dossier.py — do not edit.\n"
        "   Source of truth: experiments/fbui/dossier.template.html */\n" + css
    )
    (ROUTE / "dossier.generated.html").write_text(
        "<!-- GENERATED by experiments/fbui/build_web_dossier.py — do not edit. -->\n" + rest
    )
    leftover = re.findall(r"%%[A-Z_]+%%", css + rest)
    print(f"\ncss   {len(css)//1024}KB   html {len(rest)//1024}KB")
    print("unreplaced placeholders:", leftover or "none")


if __name__ == "__main__":
    main()
