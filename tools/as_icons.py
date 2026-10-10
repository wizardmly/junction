#!/usr/bin/env python3
"""Splits Android Studio's (IntelliJ new UI) icons into one-color layers.

GPUI draws an SVG as a mask in a single color, so each icon becomes a stack
of masks, painted in document order, each with the color its parts had.

Sources: assets/as-icons/src/<key>.svg and <key>_dark.svg, copied from
JetBrains/intellij-community (Apache 2.0). Output: assets/as-icons/gen/ and
src/ui/as_icons_gen.rs. Run from the repository root after adding an icon:
    python3 tools/as_icons.py
"""
import copy
import os
import re
import xml.etree.ElementTree as ET

SRC = "assets/as-icons/src"
GEN = "assets/as-icons/gen"
RUST = "src/ui/as_icons_gen.rs"
NS = "{http://www.w3.org/2000/svg}"
DRAWABLE = {"path", "rect", "circle", "ellipse", "line", "polyline", "polygon"}
SKIP = {"defs", "clipPath", "mask", "linearGradient", "radialGradient", "pattern", "symbol"}
NAMED = {"white": "#FFFFFF", "black": "#000000"}

ET.register_namespace("", "http://www.w3.org/2000/svg")


def tag(el):
    return el.tag.replace(NS, "")


def parse_color(value, root):
    if value is None:
        return None
    value = value.strip()
    if value == "none":
        return None
    value = NAMED.get(value, value)
    m = re.match(r"url\(#([^)]+)\)", value)
    if m:
        for el in root.iter():
            if el.get("id") == m.group(1):
                stop = next((s for s in el.iter() if tag(s) == "stop"), None)
                if stop is not None:
                    return parse_color(stop.get("stop-color", "#000000"), root)
        return (0, 0, 0)
    if value.startswith("#"):
        h = value[1:]
        if len(h) == 3:
            h = "".join(c * 2 for c in h)
        return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))
    return (0, 0, 0)


def drawables(root):
    """(element, ancestors) in paint order, outside defs/masks."""
    out = []

    def walk(el, chain):
        for child in el:
            t = tag(child)
            if t in SKIP:
                continue
            if t in DRAWABLE:
                out.append((child, chain + [child]))
            elif t == "g":
                walk(child, chain + [child])

    walk(root, [root])
    return out


def effective(chain, attr, default=None):
    for el in reversed(chain):
        if el.get(attr) is not None:
            return el.get(attr)
    return default


def layers_of(path):
    tree = ET.parse(path)
    root = tree.getroot()
    items = drawables(root)
    parts = []  # (index, "fill"/"stroke", rgba)
    for i, (el, chain) in enumerate(items):
        opacity = 1.0
        for a in chain:
            opacity *= float(a.get("opacity", "1"))
        fill = parse_color(effective(chain, "fill", "#000000"), root)
        if fill is not None:
            alpha = opacity * float(effective(chain, "fill-opacity", "1"))
            parts.append((i, "fill", fill + (round(alpha * 255),)))
        stroke = parse_color(effective(chain, "stroke"), root)
        if stroke is not None:
            alpha = opacity * float(effective(chain, "stroke-opacity", "1"))
            parts.append((i, "stroke", stroke + (round(alpha * 255),)))
    runs = []
    for part in parts:
        if runs and runs[-1][0] == part[2]:
            runs[-1][1].append(part[:2])
        else:
            runs.append((part[2], [part[:2]]))
    out = []
    for rgba, members in runs:
        layer = copy.deepcopy(tree)
        lroot = layer.getroot()
        for a in lroot.iter():
            a.attrib.pop("opacity", None)
        for i, (el, chain) in enumerate(drawables(lroot)):
            mine = {kind for (j, kind) in members if j == i}
            for a in ("fill-opacity", "stroke-opacity"):
                el.attrib.pop(a, None)
            if not mine:
                # Painted by another layer; nothing references drawables outside defs.
                chain[-2].remove(el)
                continue
            el.set("fill", "#000000" if "fill" in mine else "none")
            el.set("stroke", "#000000" if "stroke" in mine else "none")
        text = ET.tostring(lroot, encoding="unicode")
        out.append((text, rgba))
    return out


def main():
    keys = sorted(f[:-4] for f in os.listdir(SRC) if f.endswith(".svg") and not f.endswith("_dark.svg"))
    os.makedirs(GEN, exist_ok=True)
    for f in os.listdir(GEN):
        os.remove(os.path.join(GEN, f))
    consts, files = [], []
    for key in keys:
        sets = {}
        for theme, suffix in (("l", ""), ("d", "_dark")):
            src = os.path.join(SRC, f"{key}{suffix}.svg")
            if not os.path.exists(src):
                src = os.path.join(SRC, f"{key}.svg")
            entries = []
            for n, (text, rgba) in enumerate(layers_of(src)):
                name = f"{key}.{theme}{n}.svg"
                with open(os.path.join(GEN, name), "w") as fh:
                    fh.write(text)
                asset = f"as/{name}"
                files.append((asset, name))
                entries.append(f'("{asset}", 0x{rgba[0]:02x}{rgba[1]:02x}{rgba[2]:02x}{rgba[3]:02x})')
            sets[theme] = entries
        ident = re.sub(r"(?<!^)(?=[A-Z])", "_", key.replace("-", "_")).upper()
        consts.append(
            f"pub const {ident}: AsIcon = AsIcon {{ light: &[{', '.join(sets['l'])}], dark: &[{', '.join(sets['d'])}] }};"
        )
    with open(RUST, "w") as fh:
        fh.write("#![allow(dead_code)]\n")
        fh.write("// Generated by tools/as_icons.py from assets/as-icons/src; do not edit.\n")
        fh.write("// Android Studio's icons (JetBrains intellij-community, Apache 2.0), as one-color layers.\n\n")
        fh.write("use super::as_icons::AsIcon;\n\n")
        fh.write("\n".join(consts) + "\n\n")
        fh.write("/// Every layer file, served by the asset source.\n")
        fh.write("pub static FILES: &[(&str, &[u8])] = &[\n")
        for asset, name in files:
            fh.write(f'    ("{asset}", include_bytes!("../../assets/as-icons/gen/{name}")),\n')
        fh.write("];\n")
    print(f"{len(keys)} icons, {len(files)} layers")


if __name__ == "__main__":
    main()
