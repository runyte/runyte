# SPDX-License-Identifier: MPL-2.0
"""Regenerate native icons from logo/runyte_logo.svg; requires rsvg-convert."""
import base64
from pathlib import Path
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SIZES = (16, 32, 64, 128, 256, 512, 1024)


def main():
    destination = ROOT / "contrib/native/icons"
    destination.mkdir(parents=True, exist_ok=True)
    source = base64.b64encode((ROOT / "logo/runyte_logo.svg").read_bytes()).decode("ascii")
    # Keep the authored shape and charcoal fill. A light backplate makes the
    # same mark readable on both dark and light desktop panels.
    svg = f'''<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1024" height="1024" viewBox="0 0 1024 1024">
<!-- Derived from logo/runyte_logo.svg by contrib/native/render_icons.py. -->
<rect x="32" y="32" width="960" height="960" rx="196" fill="#f3f1eb"/>
<image x="144" y="144" width="736" height="736" xlink:href="data:image/svg+xml;base64,{source}"/>
</svg>
'''
    source_path = destination / "com.runyte.Runyte.svg"
    source_path.write_text(svg, encoding="utf-8")
    for size in SIZES:
        subprocess.run(["rsvg-convert", "--width", str(size), "--height", str(size),
                        "--output", str(destination / f"runyte-{size}.png"), str(source_path)], check=True)
    chunks = []
    for size, code in zip(SIZES, (b"icp4", b"icp5", b"icp6", b"ic07", b"ic08", b"ic09", b"ic10")):
        png = (destination / f"runyte-{size}.png").read_bytes()
        chunks.append(code + struct.pack(">I", len(png) + 8) + png)
    payload = b"".join(chunks)
    (destination / "Runyte.icns").write_bytes(b"icns" + struct.pack(">I", len(payload) + 8) + payload)


if __name__ == "__main__":
    main()
