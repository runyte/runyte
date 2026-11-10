#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Settled desktop insertion/deletion latency beside text or a PDF.

Uses an isolated virtual KWin/Xwayland display and targeted XSendEvent only.
The endpoint is XGetImage observing calibrated glyph pixels, not physical scanout.
Build the release desktop binary first; do not run builds during measurements.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

import editions
import plugins

REPO = Path(__file__).resolve().parent.parent
PDF = REPO / 'crates/runyte-native/src/tests/fixtures/two_pages.pdf'
ORIGINAL = ('interaction latency marker\n' + ''.join(
    f'line {number:04d}: repeated editing fixture\n' for number in range(1, 100))).encode()
COMPANION = ORIGINAL.replace(b'interaction latency marker', b'unchanged companion pane', 1)


def isolated_environment(root):
    env = plugins.environment(root)
    allowed = {'RUNYTE_TEST_STATE_HOME', 'RUNYTE_TEST_ACCOUNT_HOME',
               'RUNYTE_TEST_LOCAL_APP_DATA', 'RUNYTE_TEST_SOCKET_HOME',
               'RUNYTE_ALL_HOSTS_DIR', 'RUNYTE_CONTEXT_HOME'}
    for name in list(env):
        if name.startswith('RUNYTE_') and name not in allowed:
            del env[name]
    config = Path(env['XDG_CONFIG_HOME']) / 'runyte'
    config.mkdir()
    (config / 'config.yaml').write_text(
        'lsp:\n  enable: false\nworkspace:\n  session_strip: hidden\n')
    return env


def calibration_band(before, after, stride):
    # Only the upper-left pane's first few rows can contain the edited line.
    # Exclude status lines and the right pane entirely. edited_band then skips
    # three cells past the leftmost change, excluding both caret positions.
    width = editions.WINDOW_SIZE[0] // 2 - editions.CELL_WIDTH
    height = 8 * editions.CELL_HEIGHT
    def crop(frame):
        return b''.join(frame[y * stride:y * stride + width * 4] for y in range(height))
    return editions.edited_band(crop(before), crop(after), width * 4, height)


def settle(session):
    return editions.stable_frame(session.display, session.window,
                                 time.perf_counter() + editions.TIMEOUT, session.process)


def white_page_pixels(captured):
    """Known PDF has a white page; dark-theme error/loading panels do not.

    Require substantial white page area in the right pane, away from headers
    and borders. Retain the count as fixture-render evidence, not latency.
    """
    pixels, stride = captured
    width, height = editions.WINDOW_SIZE
    return sum(all(channel >= 235 for channel in pixels[offset:offset + 3])
               for y in range(60, height - 60)
               for x in range(width // 2 + 30, width - 30)
               for offset in [y * stride + x * 4])


def wait_shape(session, band, reference):
    session.deadline = time.perf_counter() + editions.TIMEOUT
    session.wait_for_band(band, reference, 'calibrated edited glyphs')


def save_and_verify(session, path, expected):
    session.command(f'write! {path}')
    def matches():
        try:
            return path.read_bytes() == expected
        except OSError:
            return False
    editions.poll(matches, time.perf_counter() + editions.TIMEOUT,
                  'whole-document saved verification', session.process)
    settle(session)


def drm_devices(pid):
    fields = set()
    for info in Path(f'/proc/{pid}/fdinfo').glob('*'):
        try:
            fields.update(line for line in info.read_text().splitlines()
                          if line.startswith(('drm-driver:', 'drm-pdev:')))
        except OSError:
            # Helpers can close an fd while this untimed diagnostic runs.
            continue
    return sorted(fields)


def measure_case(display, binary, mode, side, samples, spacing, paint_timing):
    with tempfile.TemporaryDirectory(prefix='ri-') as temporary:
        root = Path(temporary)
        env = isolated_environment(root)
        if paint_timing:
            env['RUNYTE_NATIVE_PAINT_TIMING'] = '1'
        project = root / 'project'
        project.mkdir()
        subprocess.run(['git', 'init', '-q', str(project)], env=display.environment(env),
                       check=True, capture_output=True)
        fixture = project / 'main.txt'
        fixture.write_bytes(ORIGINAL)
        (project / 'side.txt').write_bytes(COMPANION)
        shutil.copyfile(PDF, project / 'side.pdf')
        session = None
        result = {'mode': mode, 'side': side, 'samples': [], 'complete': False}
        try:
            session = editions.DesktopSession(display, str(binary), env, fixture,
                                              'mux-cold' if mode == 'mux' else 'ide')
            # Retain only DRM driver/device fields, never arbitrary process
            # metadata. This distinguishes hardware rendering from software.
            result['drm_devices'] = drm_devices(session.process.pid)
            settle(session)
            if mode == 'mux':
                session.command('open main.txt')
                settle(session)
            session.command(f'vsplit side.{"pdf" if side == "pdf" else "txt"}')
            split_frame = settle(session)
            if side == 'pdf':
                # Loading may itself be stable while the worker finishes.
                deadline = time.perf_counter() + editions.TIMEOUT
                while white_page_pixels(split_frame) < 10000:
                    if time.perf_counter() >= deadline:
                        raise TimeoutError('Known white PDF page did not appear in the right pane')
                    split_frame = settle(session)
                result['pdf_white_page_pixels'] = white_page_pixels(split_frame)
            for key in ('space', 'w', 'h'):
                session.key(key)
            settle(session)
            session.key('Home')
            session.key('i')
            before = settle(session)
            session.key('space')
            after = settle(session)
            band = calibration_band(before[0], after[0], before[1])
            references = [editions.mask(editions.region(frame[0], band))
                          for frame in (before, after)]
            if editions.same_shape(*references):
                raise ValueError('Insertion calibration has no distinguishable glyph change')
            session.key('BackSpace')
            restored = settle(session)
            if not editions.same_shape(editions.mask(editions.region(restored[0], band)), references[0]):
                raise ValueError('BackSpace did not restore the calibrated glyphs')
            # Start every case from an explicitly saved, clean original buffer.
            session.key('Escape')
            settle(session)
            save_and_verify(session, fixture, ORIGINAL)
            session.key('Home')
            session.key('i')
            wait_shape(session, band, references[0])
            result['band_pixels'] = band
            paint_start = (session.root / 'window.log').stat().st_size
            for index in range(samples):
                pair = {'index': index + 1}
                for name, key, reference in [('insert_ms', 'space', references[1]),
                                              ('delete_ms', 'BackSpace', references[0])]:
                    time.sleep(spacing)
                    started = time.perf_counter()
                    session.key(key)
                    wait_shape(session, band, reference)
                    pair[name] = (time.perf_counter() - started) * 1000
                result['samples'].append(pair)
            if paint_timing:
                measured_log = (session.root / 'window.log').read_bytes()[paint_start:].decode(errors='replace')
                result['native_paint_trace'] = [line for line in measured_log.splitlines()
                                                if line.startswith('native-paint ')]
            session.key('Escape')
            settle(session)
            save_and_verify(session, project / 'restored.txt', ORIGINAL)
            # An additional real insertion/save proves the calibrated change was
            # document text, not a mode indicator or cursor-only animation.
            session.key('Home')
            session.key('i')
            session.key('space')
            wait_shape(session, band, references[1])
            session.key('Escape')
            settle(session)
            save_and_verify(session, project / 'inserted.txt', b' ' + ORIGINAL)
            result['verified_restored_and_inserted'] = True
            result['insert_ms'] = plugins.summary([p['insert_ms'] for p in result['samples']], samples)
            result['delete_ms'] = plugins.summary([p['delete_ms'] for p in result['samples']], samples)
            for name in ('insert_ms', 'delete_ms'):
                ordered = sorted(pair[name] for pair in result['samples'])
                result[name]['p99'] = ordered[math.ceil(len(ordered) * .99) - 1]
            # q! closes just the active pane; this fixture owns two panes.
            session.key('Escape')
            time.sleep(0.08)
            session.exit_after(lambda: session.command('detach' if mode == 'mux' else 'qa!'),
                               editions.DETACH_TIMEOUT_SECONDS if mode == 'mux' else 10)
            result['complete'] = True
        except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
            result['error'] = str(error).replace(str(root), '<fixture>')
        finally:
            if session is not None:
                session.log.flush()
                log = (session.root / 'window.log').read_text(errors='replace')
                if not result['complete']:
                    result['log_tail'] = log[-8000:].replace(str(root), '<fixture>')
                session.close()
            if mode == 'mux':
                cleanup = subprocess.run(
                    [str(binary), '--session-stop', '--force', str(project)],
                    env=display.environment(env), cwd=project, capture_output=True, timeout=20)
                if cleanup.returncode:
                    result['complete'] = False
                    result['cleanup_error'] = cleanup.stderr.decode(errors='replace')[-2000:].replace(str(root), '<fixture>')
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=REPO / 'target/release/runyte-desktop')
    parser.add_argument('--samples', type=int, default=40, help='insert/delete pairs per case')
    parser.add_argument('--rounds', type=int, default=1)
    parser.add_argument('--spacing', type=float, default=0.2, help='untimed seconds before each key')
    parser.add_argument('--modes', default='ide,mux')
    parser.add_argument('--sides', default='text,pdf')
    parser.add_argument('--paint-timing', action='store_true', help='diagnostic timing adds stderr overhead')
    parser.add_argument('--max-busy-cores', type=float, default=1.0)
    parser.add_argument('--quiet-timeout', type=float, default=60.0)
    parser.add_argument('--json', type=Path, required=True)
    args = parser.parse_args()
    modes, sides = args.modes.split(','), args.sides.split(',')
    if (args.samples < 1 or args.rounds < 1 or not math.isfinite(args.spacing) or args.spacing < 0.05
            or not math.isfinite(args.max_busy_cores) or args.max_busy_cores <= 0
            or not math.isfinite(args.quiet_timeout) or args.quiet_timeout <= 0
            or set(modes) - {'ide', 'mux'} or set(sides) - {'text', 'pdf'}):
        parser.error('positive counts, spacing >= .05, modes ide/mux and sides text/pdf required')
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error('Build the desktop release binary first')
    args.json.parent.mkdir(parents=True, exist_ok=True)
    output = {'complete': False, 'metric': 'XSendEvent to calibrated glyphs observed by XGetImage; not physical scanout',
              'platform': platform.platform(),
              'geometry_cells': [120, 40], 'spacing_seconds': args.spacing,
              'paint_timing': args.paint_timing, 'samples_per_case': args.samples,
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'cases': []}
    try:
        with tempfile.TemporaryDirectory(prefix='ri-display-') as temporary:
            display = editions.Display(isolated_environment(Path(temporary)))
            try:
                cases = [(mode, side) for mode in modes for side in sides]
                for round_index in range(args.rounds):
                    for offset in range(len(cases)):
                        mode, side = cases[(offset + round_index) % len(cases)]
                        quiet = editions.wait_until_quiet(args.max_busy_cores, args.quiet_timeout)
                        if not quiet['quiet']:
                            raise RuntimeError(f'Machine did not become quiet: {quiet}')
                        print(f'Measuring {mode}/{side}, round {round_index + 1}', file=sys.stderr, flush=True)
                        result = measure_case(display, binary, mode, side, args.samples,
                                              args.spacing, args.paint_timing)
                        output['cases'].append({'round': round_index + 1, 'quiet_before': quiet, **result})
                        args.json.write_text(json.dumps(output, indent=2) + '\n')
            finally:
                display.close()
        output['complete'] = bool(output['cases']) and all(case['complete'] for case in output['cases'])
    finally:
        args.json.write_text(json.dumps(output, indent=2) + '\n')
    return 0 if output['complete'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
