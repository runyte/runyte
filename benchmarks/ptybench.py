# SPDX-License-Identifier: MPL-2.0

"""Pseudo-terminal harness for measuring terminal editor startup and idle cost.

Editors are spawned on a real pty at a fixed size and driven the way a terminal
emulator would drive them. Two properties of that environment matter enough to
be part of the measurement rather than incidental to it:

Terminal capability queries must be answered. Every editor measured here emits
some combination of DA1, DA2, the kitty keyboard query, DECRQM mode queries,
DSR and OSC colour requests before or during its first draw, and at least one
of them will not draw at all until it receives a reply. A harness that stays
silent measures that stall instead of the editor.

Keystrokes must be staggered. Writing ``ESC : q ! CR`` as one block is parsed by
crossterm-based editors as Alt-``:``, because the escape and the byte after it
arrive in the same read. Sending the escape alone and pausing before the rest
is what a human keyboard looks like.

Both were originally measured wrong. The first produced a fabricated one-second
quit cost for one editor; the second produced an apparent hang. Neither was a
property of the editor under test.
"""

from __future__ import annotations

import fcntl
import os
import pty
import re
import select
import signal
import statistics
import struct
import termios
import time

# Silence long enough to call the initial draw finished. Chunks below
# SIGNIFICANT_BYTES do not restart this clock, so an editor that repaints a
# cursor on a timer still reaches a settled state.
SETTLE_SECONDS = 0.25
SIGNIFICANT_BYTES = 256

# Upper bound on one startup measurement. Generous: the largest fixture takes
# about a second and this only has to stop a genuinely stuck process.
STARTUP_TIMEOUT_SECONDS = 40.0
QUIT_TIMEOUT_SECONDS = 10.0

ROWS, COLUMNS = 40, 120

# ESC alone, then the command, so the escape is not folded into an Alt chord.
QUIT_SEQUENCE = ((b"\x1b", 0.08), (b":", 0.05), (b"q!", 0.05), (b"\r", 0.0))
QUIT_KEYSTROKE_COST = sum(gap for _, gap in QUIT_SEQUENCE)


def terminal_replies(chunk: bytes) -> bytes:
    """Answer the capability queries in `chunk` as a modern xterm would.

    Only queries that an editor may block on are answered. Anything else is
    ignored, which is also what a real terminal does with a request it does not
    implement.
    """
    reply = b""
    reply += b"\x1b[?62;1;2;6;9;15;22c" * chunk.count(b"\x1b[c")           # DA1
    reply += b"\x1b[>0;276;0c" * chunk.count(b"\x1b[>c")                   # DA2
    reply += b"\x1b[?0u" * chunk.count(b"\x1b[?u")                         # kitty keyboard
    reply += b"\x1b[0n" * chunk.count(b"\x1b[5n")                          # DSR status
    reply += b"\x1b[1;1R" * chunk.count(b"\x1b[6n")                        # DSR cursor
    reply += b"\x1bP>|xterm(390)\x1b\\" * chunk.count(b"\x1b[>0q")         # XTVERSION
    for mode in re.finditer(rb"\x1b\[\?(\d+)\$p", chunk):                  # DECRQM
        reply += b"\x1b[?" + mode.group(1) + b";2$y"
    reply += b"\x1bP0$r\x1b\\" * len(re.findall(rb"\x1bP\$q", chunk))      # DECRQSS
    for osc in re.finditer(rb"\x1b\](\d+);\?(?:\x07|\x1b\\)", chunk):      # OSC colour
        reply += b"\x1b]" + osc.group(1) + b";rgb:0000/0000/0000\x07"
    return reply


def _configure(fd: int) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLUMNS, 0, 0))
    try:
        attributes = termios.tcgetattr(fd)
        attributes[3] &= ~termios.ECHO
        termios.tcsetattr(fd, termios.TCSANOW, attributes)
    except termios.error:
        pass


def _spawn(argv: list[str], env: dict[str, str], cwd: str | None) -> tuple[int, int]:
    pid, fd = pty.fork()
    if pid == 0:
        if cwd:
            os.chdir(cwd)
        os.environ.update(env)
        os.environ["TERM"] = "xterm-256color"
        try:
            os.execvp(argv[0], argv)
        except OSError:
            os._exit(127)
    _configure(fd)
    return pid, fd


def _reap(pid: int) -> None:
    try:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    except (ProcessLookupError, ChildProcessError):
        pass


def measure_startup(argv, env, cwd=None):
    """Open a document and quit. Returns first-paint, ready, quit and byte count.

    ``first_paint`` is the first byte of output; ``ready`` is when output goes
    quiet. ``quit`` has the harness's own keystroke stagger subtracted, so a
    value near zero means the editor exited as soon as it read the command.
    """
    pid, fd = _spawn(argv, env, cwd)
    start = time.perf_counter()
    first_paint = None
    last_significant = start
    total_bytes = 0
    ready = None
    closed = False

    while time.perf_counter() - start < STARTUP_TIMEOUT_SECONDS:
        readable, _, _ = select.select([fd], [], [], 0.01)
        if readable:
            try:
                data = os.read(fd, 65536)
            except OSError:
                closed = True
                break
            if not data:
                closed = True
                break
            if first_paint is None:
                first_paint = time.perf_counter() - start
                # Start the settle clock at the first byte, not at spawn, or an
                # editor that queries the terminal before drawing settles at zero.
                last_significant = time.perf_counter()
            total_bytes += len(data)
            if len(data) >= SIGNIFICANT_BYTES:
                last_significant = time.perf_counter()
            reply = terminal_replies(data)
            if reply:
                try:
                    os.write(fd, reply)
                except OSError:
                    pass
        elif first_paint is not None and time.perf_counter() - last_significant > SETTLE_SECONDS:
            ready = last_significant - start
            break

    quit_start = time.perf_counter()
    if not closed:
        for part, gap in QUIT_SEQUENCE:
            try:
                os.write(fd, part)
            except OSError:
                break
            time.sleep(gap)

    exited = None
    saw_eof = False
    while time.perf_counter() - quit_start < QUIT_TIMEOUT_SECONDS:
        readable, _, _ = select.select([fd], [], [], 0.01)
        if readable:
            try:
                data = os.read(fd, 65536)
                if not data:
                    saw_eof = True
                else:
                    # Editors query the terminal while tearing down too; an
                    # unanswered query here looks like a slow exit.
                    reply = terminal_replies(data)
                    if reply:
                        os.write(fd, reply)
            except OSError:
                saw_eof = True
        try:
            reaped, _ = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            break
        if reaped == pid:
            exited = time.perf_counter() - quit_start
            break
        if saw_eof:
            try:
                os.waitpid(pid, 0)
            except ChildProcessError:
                pass
            exited = time.perf_counter() - quit_start
            break

    if exited is None:
        _reap(pid)
    try:
        os.close(fd)
    except OSError:
        pass

    return {
        "first_paint": first_paint,
        "ready": ready,
        "quit": None if exited is None else max(0.0, exited - QUIT_KEYSTROKE_COST),
        "bytes": total_bytes,
    }


def _cpu_ticks(pid: int) -> int:
    """User + system ticks for `pid` and every descendant it has spawned."""
    total = 0
    try:
        fields = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[-1].split()
        total += int(fields[11]) + int(fields[12])
    except (OSError, IndexError, ValueError):
        return total
    try:
        for task in os.listdir(f"/proc/{pid}/task"):
            for child in open(f"/proc/{pid}/task/{task}/children").read().split():
                total += _cpu_ticks(int(child))
    except (OSError, ValueError):
        pass
    return total


def measure_idle(argv, env, cwd=None, settle=2.5, window=10.0):
    """Open a document, wait for startup to finish, then watch an idle editor.

    Reports CPU percentage over `window` and how many times the editor wrote to
    the screen without any input. A fully event-driven editor reports zero for
    both.
    """
    pid, fd = _spawn(argv, env, cwd)
    start = time.time()
    while time.time() - start < settle:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            reply = terminal_replies(data)
            if reply:
                os.write(fd, reply)

    before = _cpu_ticks(pid)
    window_start = time.time()
    writes = 0
    idle_bytes = 0
    while time.time() - window_start < window:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            writes += 1
            idle_bytes += len(data)
            reply = terminal_replies(data)
            if reply:
                os.write(fd, reply)
    elapsed = time.time() - window_start
    ticks = _cpu_ticks(pid) - before
    _reap(pid)
    try:
        os.close(fd)
    except OSError:
        pass

    hertz = os.sysconf("SC_CLK_TCK")
    return {
        "cpu_percent": ticks / hertz / elapsed * 100.0,
        "writes": writes,
        "bytes": idle_bytes,
    }


def median_startup(argv, env, cwd=None, runs=5):
    """Median of `runs` startup measurements, with the sample count kept."""
    samples = [measure_startup(argv, env, cwd) for _ in range(runs)]

    def median_ms(key):
        values = [s[key] for s in samples if s[key] is not None]
        return round(statistics.median(values) * 1000, 1) if values else None

    return {
        "first_paint_ms": median_ms("first_paint"),
        "ready_ms": median_ms("ready"),
        "quit_ms": median_ms("quit"),
        "bytes": int(statistics.median(s["bytes"] for s in samples)),
        "runs": runs,
        "complete": sum(1 for s in samples if s["ready"] is not None),
    }
