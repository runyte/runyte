#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Compare pane-row painting against a baseline on isolated KWin/Xwayland.

Targeted XSendEvent only. Fixed body pixels exclude changing window/pane titles
and status lines. Both fresh scene construction and a replay after an overlay
are compared. Output contains hashes/counts, never fixture paths or screenshots.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import struct
import zlib
import tempfile
import time

import editions
import interaction_latency as latency


def normal(session):
    latency.terminal_normal(session)
    latency.settle(session)


def capture(session):
    pixels, stride = latency.settle(session)
    return b''.join(pixels[y * stride + 9 * 4:y * stride + 1070 * 4]
                    for y in range(40, 720))


def run(display, binary, root, side):
    env = latency.isolated_environment(root)
    env['SHELL'] = '/bin/sh'
    env['PS1'] = ''
    project = root / 'project'
    project.mkdir()
    subprocess.run(['git', 'init', '-q', str(project)], env=env, check=True)
    fixture = project / 'main.txt'
    fixture.write_bytes(latency.ORIGINAL)
    (project / 'side.txt').write_bytes(latency.COMPANION)
    shutil.copyfile(latency.PDF, project / 'side.pdf')
    (project / 'side.html').write_text('<html><body style="margin:0;background:white">'
        '<div style="height:300px;background:#285ac8"></div></body></html>')
    png(project / 'side.png', bytes([255, 255, 255, 255]) * (1061 * 680))
    sample = 'ab == ffi \ue0b0 \uf120 界 e\u0301 😀🌍'
    styles = ['0', '1', '3', '1;3', '2', '7', '8', '4', '9', '4;9']
    (project / 'styles.ansi').write_text('\x1b[2J\x1b[H' + ''.join(
        '\x1b[0m' + str(index).ljust(4) + '\x1b[38;2;255;80;90m\x1b[' + style + 'm'
        + ('\x1b[48;2;24;24;24m' if index % 2 else '\x1b[48;2;11;33;44m')
        + sample + '\x1b[0m\r\n' for index, style in enumerate(styles * 2))
        + ''.join(f'\x1b[{row};56H\x1b[3;4m\uf120😀\x1b[0m'
                  for row in range(1, 21)) + '\x1b[14;59H')
    session = editions.DesktopSession(display, str(binary), env, fixture, 'ide')
    try:
        latency.settle(session)
        session.command(f'vsplit side.{side}')
        frame = latency.settle(session)
        if side == 'html':
            session.command('preview')
            frame = latency.settle(session)
        if side in ('pdf', 'png', 'html'):
            deadline = time.perf_counter() + editions.TIMEOUT
            while latency.white_page_pixels(frame) < 10000:
                if time.perf_counter() > deadline:
                    raise TimeoutError(f'{side} media/preview missing')
                frame = latency.settle(session)
        for key in ('space', 'w', 'h'):
            session.key(key)
        latency.settle(session)
        session.command('terminal')
        latency.settle(session)
        display.text(session.window, 'cat styles.ansi')
        session.key('Return')
        time.sleep(.5)
        shots = {'styled': capture(session)}
        # Prove that styled output, rather than an empty shell, is compared.
        assert shots['styled'].count(bytes([90, 80, 255, 255])) > 100
        normal(session)
        for key in ('space', 'f'):
            session.key(key)
        shots['overlay'] = capture(session)
        session.key('Escape')
        shots['replayed'] = capture(session)
        # The finder must cover the companion pane too; otherwise an input
        # routing failure would silently skip the media-overlap case.
        row_bytes = 1061 * 4
        changed_right = sum(
            shots['overlay'][offset:offset + 4] != shots['replayed'][offset:offset + 4]
            for y in range(680) for x in range(600, 1000)
            for offset in [y * row_bytes + (x - 9) * 4])
        assert changed_right > 1000, 'Finder did not visibly cover the companion pane'
        session.exit_after(lambda: session.command('qa!!'), 10)
        return shots
    finally:
        session.close()


def png(path, pixels):
    def chunk(kind, data):
        return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data) & 0xffffffff)
    rows = b''.join(b'\0' + b''.join(pixels[offset:offset + 3][::-1]
        for offset in range(y * 1061 * 4, (y + 1) * 1061 * 4, 4)) for y in range(680))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!2I5B', 1061, 680, 8, 2, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--json', type=Path, required=True)
    args = parser.parse_args()
    output = {'complete': False, 'cases': [], 'binary_sha256': {
        name: hashlib.sha256(binary.read_bytes()).hexdigest()
        for name, binary in [('before', args.before), ('after', args.after)]}}
    with tempfile.TemporaryDirectory(prefix='pane-pixels-') as temp:
        root = Path(temp)
        display_root = root / 'display'
        display_root.mkdir()
        display = editions.Display(latency.isolated_environment(display_root))
        try:
            for side in ('txt', 'pdf', 'png', 'html'):
                shots = []
                for label, binary in [('before', args.before), ('after', args.after)]:
                    fixture_root = root / side
                    if fixture_root.exists():
                        shutil.rmtree(fixture_root)
                    fixture_root.mkdir()
                    shots.append(run(display, binary.resolve(), fixture_root, side))
                for scene in shots[0]:
                    left, right = shots[0][scene], shots[1][scene]
                    differences = sum(left[x:x + 4] != right[x:x + 4]
                                      for x in range(0, len(left), 4))
                    if differences:
                        png(args.json.with_name(f'{side}-{scene}-before.png'), left)
                        png(args.json.with_name(f'{side}-{scene}-after.png'), right)
                    output['cases'].append({'side': side, 'scene': scene,
                        'different_pixels': differences,
                        'before_sha256': hashlib.sha256(left).hexdigest(),
                        'after_sha256': hashlib.sha256(right).hexdigest()})
        finally:
            display.close()
    output['complete'] = all(case['different_pixels'] == 0 for case in output['cases'])
    args.json.write_text(json.dumps(output, indent=2) + '\n')
    print(json.dumps(output, indent=2))
    return 0 if output['complete'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
