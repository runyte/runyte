#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Real bounded PDF helper acceptance; --compare measures authored PDFs vs Poppler."""
import argparse
from collections import Counter
import hashlib
import json
import re
import xml.etree.ElementTree as ET
import os
from pathlib import Path
import statistics
import struct
import subprocess
import tempfile
import time


def pdf(objects, extra=b""):
    data = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for n, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f"{n} 0 obj\n".encode() + obj + b"\nendobj\n")
    start = len(data)
    data.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        data.extend(f"{offset:010} 00000 n \n".encode())
    data.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R ".encode()
                + extra + f">>\nstartxref\n{start}\n%%EOF\n".encode())
    return data


def stream(data, attributes=b""):
    return b"<< /Length " + str(len(data)).encode() + b" " + attributes + b">>\nstream\n" + data + b"\nendstream"


def objects(content, options=b"", resources=b"", font=b"Helvetica"):
    return [b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] " + options
            + b" /Resources << /Font << /F1 4 0 R >> " + resources + b">> /Contents 5 0 R >>",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /" + font + b" >>",
            stream(content)]


def rc4(key, data):
    state = list(range(256))
    j = 0
    for i in range(256):
        j = (j + state[i] + key[i % len(key)]) % 256
        state[i], state[j] = state[j], state[i]
    i = j = 0
    out = bytearray()
    for byte in data:
        i = (i + 1) % 256
        j = (j + state[i]) % 256
        state[i], state[j] = state[j], state[i]
        out.append(byte ^ state[(state[i] + state[j]) % 256])
    return out


def encrypted(password):
    padding = bytes.fromhex("28bf4e5e4e758a4164004e56fffa01082e2e00b6d0683e802f0ca9fe6453697a")
    pad = lambda value: (value + padding)[:32]
    owner = rc4(hashlib.md5(pad(b"fixture-owner")).digest()[:5], pad(password))
    identity = b"runyte-pdf-tests"
    key = hashlib.md5(pad(password) + owner + struct.pack("<i", -4) + identity).digest()[:5]
    user = rc4(key, padding)
    content = b"BT /F1 20 Tf 40 100 Td (Secret) Tj ET"
    obj = objects(content)
    obj[4] = stream(rc4(hashlib.md5(key + b"\x05\0\0\0\0").digest()[:10], content))
    obj.append(b"<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 /O <" + owner.hex().encode()
               + b"> /U <" + user.hex().encode() + b"> >>")
    return pdf(obj, b"/Encrypt 6 0 R /ID [<" + identity.hex().encode() + b"><" + identity.hex().encode() + b">] ")


def fixtures():
    text = b"BT /F1 20 Tf 50 80 Td (Hello World) Tj ET"
    cases = {"text": pdf(objects(text)),
             "columns": pdf(objects(b"BT /F1 12 Tf 20 170 Td (Left One) Tj 0 -20 Td (Left Two) Tj ET BT /F1 12 Tf 180 170 Td (Right One) Tj 0 -20 Td (Right Two) Tj ET")),
             "vector": pdf(objects(b"0.2 0.4 0.8 rg 40 60 80 70 re f " + text)),
             "empty-password": encrypted(b"")}
    for rotation in (0, 90, 180, 270):
        cases[f"crop-{rotation}"] = pdf(objects(text, f"/CropBox [20 30 280 180] /Rotate {rotation}".encode()))
    image = objects(b"q 200 0 0 140 40 30 cm /Im1 Do Q", resources=b"/XObject << /Im1 6 0 R >>")
    image.append(stream(bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]),
                        b"/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 "))
    cases["scan"] = pdf(image)
    form = objects(b"q 1 0 0 1 10 10 cm /Fm1 Do Q", resources=b"/XObject << /Fm1 6 0 R >>")
    form.append(stream(text, b"/Type /XObject /Subtype /Form /BBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> "))
    cases["form"] = pdf(form)
    return cases


def helper(binary, path, root, *geometry):
    env = dict(os.environ, PATH=str(root / "empty-path"), XDG_CONFIG_HOME=str(root / "config"))
    start = time.perf_counter()
    result = subprocess.run([str(binary), "--native-pdf-helper", str(path), "1", *map(str, geometry)],
                            env=env, capture_output=True, timeout=20, check=True)
    elapsed = time.perf_counter() - start
    assert result.stdout[:8] == b"RYTPDF01", result.stderr
    size = struct.unpack("<I", result.stdout[8:12])[0]
    header = json.loads(result.stdout[12:12 + size])
    pixels = result.stdout[12 + size:]
    if "Ok" in header:
        meta = header["Ok"]
        assert len(pixels) == meta["width"] * meta["height"] * 4
        assert meta["pages"] >= 1
        assert all(pixels[i] == 255 for i in range(3, len(pixels), 4))
    return header, pixels, elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/runyte"))
    parser.add_argument("--compare", action="store_true")
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="runyte-pdf-") as directory:
        root = Path(directory)
        (root / "empty-path").mkdir()
        for name, data in fixtures().items():
            path = root / f"{name}.pdf"
            path.write_bytes(data)
            header, pixels, elapsed = helper(binary, path, root)
            assert "Ok" in header, (name, header)
            meta = header["Ok"]
            assert meta["text_error"] is None, (name, meta)
            words = " ".join(w["text"] for w in meta["words"])
            expected = "" if name == "scan" else "Secret" if name == "empty-password" else "Left One Left Two Right One Right Two" if name == "columns" else "Hello World"
            assert words == expected, (name, words)
            detail, _, _ = helper(binary, path, root, 524288, 524288, 100, 100, 32, 32)
            assert "Ok" in detail, detail
            if args.compare:
                hayro_times = [elapsed] + [helper(binary, path, root)[2] for _ in range(2)]
                poppler_times = []
                for _ in range(3):
                    start = time.perf_counter()
                    for command in (["pdfinfo", str(path)],
                                    ["pdftoppm", "-cropbox", "-singlefile", "-scale-to", "1600", "-png", str(path), str(root / "poppler")],
                                    ["pdftotext", "-cropbox", "-bbox-layout", str(path), str(root / "words.xml")]):
                        subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True, timeout=20)
                    poppler_times.append(time.perf_counter() - start)
                page_words = ET.parse(root / "words.xml").getroot().iter()
                poppler_words = " ".join(n.text or "" for n in page_words if n.tag.rsplit("}", 1)[-1] == "word")
                assert Counter(poppler_words.split()) == Counter(expected.split()), (name, poppler_words)
                order = "same" if poppler_words == expected else "differs"
                # Align physical dimensions explicitly: the renderers round the
                # short base axis differently. Compare every sixteenth RGB pixel.
                subprocess.run(["pdftoppm", "-cropbox", "-singlefile", "-scale-dimension-before-rotation",
                                "-scale-to-x", str(meta["width"]), "-scale-to-y", str(meta["height"]),
                                str(path), str(root / "comparison")], check=True, capture_output=True)
                ppm = (root / "comparison.ppm").read_bytes()
                match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", ppm)
                assert match and (int(match[1]), int(match[2])) == (meta["width"], meta["height"])
                rgb = ppm[match.end():]
                assert len(rgb) == len(pixels) // 4 * 3
                errors = [abs(pixels[i * 4 + c] - rgb[i * 3 + c])
                          for i in range(0, len(rgb) // 3, 16) for c in range(3)]
                mae = statistics.mean(errors)
                assert mae < 8, (name, mae)
                detail_times = [helper(binary, path, root, 3200, 2400, 200, 200, 600, 600)[2] for _ in range(3)]
                poppler_detail = []
                for _ in range(3):
                    start = time.perf_counter()
                    subprocess.run(["pdftoppm", "-cropbox", "-singlefile", "-scale-dimension-before-rotation",
                                    "-scale-to-x", "3200", "-scale-to-y", "2400", "-x", "200", "-y", "200",
                                    "-W", "600", "-H", "600", "-png", str(path), str(root / "detail")],
                                   check=True, capture_output=True)
                    poppler_detail.append(time.perf_counter() - start)
                print(f"{name}: base Hayro {statistics.median(hayro_times)*1000:.1f} / Poppler {statistics.median(poppler_times)*1000:.1f} ms; "
                      f"detail {statistics.median(detail_times)*1000:.1f} / {statistics.median(poppler_detail)*1000:.1f} ms; RGB MAE {mae:.3f}; word order {order}", flush=True)
        for name, data in {"password": encrypted(b"secret"), "damaged": b"not a PDF",
                           "font": pdf(objects(b"BT /F1 20 Tf 50 80 Td (Hello) Tj ET", font=b"UninstalledTestFont"))}.items():
            path = root / f"{name}.pdf"
            path.write_bytes(data)
            header, _, _ = helper(binary, path, root)
            assert "Err" in header, (name, header)
            if name == "password":
                assert "password" in header["Err"], header
        print("Native PDF helper acceptance passed (Poppler absent from PATH).")


if __name__ == "__main__":
    main()
