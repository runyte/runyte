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

# Silence long enough to classify document output as settled. The caller also
# supplies a token from the first visible document line; capability exchanges,
# terminal setup, and loading presentations cannot settle without that evidence.
# Every byte after the marker restarts this clock, independent of pty read chunk
# boundaries. The quiet interval confirms settlement but is not added to the
# reported timestamp.
SETTLE_SECONDS = 0.25

# Upper bound on one startup measurement. Generous: the largest fixture takes
# about a second and this only has to stop a genuinely stuck process.
STARTUP_TIMEOUT_SECONDS = 40.0
QUIT_TIMEOUT_SECONDS = 10.0

ROWS, COLUMNS = 40, 120

# ESC alone, then the command, so the escape is not folded into an Alt chord.
QUIT_SEQUENCE = ((b"\x1b", 0.08), (b":", 0.05), (b"q!", 0.05), (b"\r", 0.0))


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


def measure_startup(argv, env, document_marker, cwd=None):
    """Open a document and quit. Return first-byte, settled-output and exit data.

    ``first_byte`` is any first byte written to the pty. It may be a capability
    query, terminal setup, or a loading presentation, so it is diagnostic rather
    than a cross-editor readiness measurement. ``first_document_output`` is when
    a token from the shared first document line is observed. ``settled_output``
    is the last output before the quiet window and is used only to avoid sending
    quit during startup drawing. ``quit`` begins after the final keystroke, so a
    value near zero means the editor exited as soon as it read the command.
    """
    if not document_marker:
        raise ValueError("document marker must not be empty")

    start = time.perf_counter()
    pid, fd = _spawn(argv, env, cwd)
    first_byte = None
    last_document_output = None
    total_bytes = 0
    document_seen = False
    first_document_output = None
    marker_tail = b""
    settled_output = None
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
            if first_byte is None:
                first_byte = time.perf_counter() - start
            total_bytes += len(data)
            output_at = time.perf_counter()
            if document_seen:
                last_document_output = output_at
            else:
                marker_input = marker_tail + data
                if document_marker in marker_input:
                    document_seen = True
                    first_document_output = output_at - start
                    last_document_output = output_at
                tail_length = len(document_marker) - 1
                marker_tail = marker_input[-tail_length:] if tail_length else b""
            reply = terminal_replies(data)
            if reply:
                try:
                    os.write(fd, reply)
                except OSError:
                    pass
        elif (
            document_seen
            and last_document_output is not None
            and time.perf_counter() - last_document_output > SETTLE_SECONDS
        ):
            settled_output = last_document_output - start
            break

    quit_command_at = None
    if not closed:
        for part, gap in QUIT_SEQUENCE:
            try:
                os.write(fd, part)
            except OSError:
                break
            if gap:
                time.sleep(gap)
        else:
            quit_command_at = time.perf_counter()

    exit_wait_start = time.perf_counter()
    exited = None
    exit_status = None
    saw_eof = False
    while time.perf_counter() - exit_wait_start < QUIT_TIMEOUT_SECONDS:
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
            reaped, status = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            break
        if reaped == pid:
            exited = time.perf_counter()
            exit_status = status
            break
        if saw_eof:
            try:
                _, exit_status = os.waitpid(pid, 0)
            except ChildProcessError:
                pass
            exited = time.perf_counter()
            break

    if exited is None:
        _reap(pid)
    try:
        os.close(fd)
    except OSError:
        pass

    return {
        "first_byte": first_byte,
        "first_document_output": first_document_output,
        "settled_output": settled_output,
        "quit": (
            None
            if (
                exited is None
                or settled_output is None
                or quit_command_at is None
                or exit_status is None
                or not os.WIFEXITED(exit_status)
                or os.WEXITSTATUS(exit_status) != 0
            )
            else max(0.0, exited - quit_command_at)
        ),
        "bytes": total_bytes,
    }


def _stat_cpu_ticks(stat: str) -> int:
    """Own and reaped-child CPU ticks from one Linux `/proc/PID/stat`."""
    fields = stat.rsplit(")", 1)[-1].split()
    # Fields 14-17 are utime, stime, cutime and cstime. `fields` starts at
    # process state (field 3), so their zero-based positions here are 11-14.
    return sum(int(fields[index]) for index in range(11, 15))


def _cpu_ticks(pid: int) -> int:
    """User + system ticks for `pid` and every descendant it has spawned.

    A live child is traversed through `/proc`; a child reaped between samples
    remains represented by its parent's cumulative child ticks. Combining the
    two keeps short-lived helpers inside the measured window without counting
    a live child twice.
    """
    total = 0
    try:
        with open(f"/proc/{pid}/stat") as stat_file:
            total += _stat_cpu_ticks(stat_file.read())
    except (OSError, IndexError, ValueError):
        return total
    try:
        for task in os.listdir(f"/proc/{pid}/task"):
            with open(f"/proc/{pid}/task/{task}/children") as children_file:
                children = children_file.read().split()
            for child in children:
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
    complete = True
    start = time.perf_counter()
    while time.perf_counter() - start < settle:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                data = os.read(fd, 65536)
            except OSError:
                complete = False
                break
            if not data:
                complete = False
                break
            reply = terminal_replies(data)
            if reply:
                try:
                    os.write(fd, reply)
                except OSError:
                    complete = False
                    break

    cpu_supported = os.path.exists(f"/proc/{pid}/stat")
    before = _cpu_ticks(pid) if cpu_supported else 0
    window_start = time.perf_counter()
    writes = 0
    idle_bytes = 0
    while complete and time.perf_counter() - window_start < window:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                data = os.read(fd, 65536)
            except OSError:
                complete = False
                break
            if not data:
                complete = False
                break
            writes += 1
            idle_bytes += len(data)
            reply = terminal_replies(data)
            if reply:
                try:
                    os.write(fd, reply)
                except OSError:
                    complete = False
                    break
    elapsed = time.perf_counter() - window_start
    ticks = _cpu_ticks(pid) - before if cpu_supported else 0
    try:
        reaped, _ = os.waitpid(pid, os.WNOHANG)
        if reaped == pid:
            complete = False
    except ChildProcessError:
        complete = False
    _reap(pid)
    try:
        os.close(fd)
    except OSError:
        pass

    hertz = os.sysconf("SC_CLK_TCK")
    return {
        "cpu_percent": (
            ticks / hertz / elapsed * 100.0 if complete and cpu_supported else None
        ),
        "writes": writes,
        "bytes": idle_bytes,
        "complete": complete,
    }


def median_idle(argv, env, cwd=None, runs=5, settle=2.5, window=10.0):
    """Median of complete idle windows, refusing partial result sets."""
    samples = [
        measure_idle(argv, env, cwd, settle=settle, window=window)
        for _ in range(runs)
    ]
    complete = sum(1 for sample in samples if sample["complete"])
    if complete != runs:
        return {
            "cpu_percent": None,
            "cpu_min": None,
            "cpu_max": None,
            "writes": None,
            "writes_min": None,
            "writes_max": None,
            "bytes": None,
            "runs": runs,
            "complete": complete,
        }

    cpu_values = [
        sample["cpu_percent"]
        for sample in samples
        if sample["cpu_percent"] is not None
    ]
    return {
        "cpu_percent": (
            statistics.median(cpu_values) if len(cpu_values) == runs else None
        ),
        "cpu_min": min(cpu_values) if len(cpu_values) == runs else None,
        "cpu_max": max(cpu_values) if len(cpu_values) == runs else None,
        "writes": statistics.median(sample["writes"] for sample in samples),
        "writes_min": min(sample["writes"] for sample in samples),
        "writes_max": max(sample["writes"] for sample in samples),
        "bytes": statistics.median(sample["bytes"] for sample in samples),
        "runs": runs,
        "complete": complete,
    }


def median_startup(argv, env, document_marker, cwd=None, runs=5):
    """Median of `runs` startup and quit measurements, with completeness kept."""
    samples = [
        measure_startup(argv, env, document_marker, cwd) for _ in range(runs)
    ]

    def median_ms(key, valid=lambda sample: True):
        values = [s[key] for s in samples if s[key] is not None and valid(s)]
        return round(statistics.median(values) * 1000, 1) if values else None

    def valid_document_startup(sample):
        return (
            sample["first_document_output"] is not None
            and sample["settled_output"] is not None
        )

    return {
        "first_byte_ms": median_ms("first_byte", valid_document_startup),
        "first_document_output_ms": median_ms(
            "first_document_output", valid_document_startup
        ),
        "settled_output_ms": median_ms("settled_output"),
        "quit_ms": median_ms("quit"),
        "bytes": int(statistics.median(s["bytes"] for s in samples)),
        "runs": runs,
        "first_byte_complete": sum(
            1 for s in samples if s["first_byte"] is not None and valid_document_startup(s)
        ),
        "first_document_output_complete": sum(
            1 for s in samples if valid_document_startup(s)
        ),
        "settled_output_complete": sum(
            1 for s in samples if s["settled_output"] is not None
        ),
        "quit_complete": sum(1 for s in samples if s["quit"] is not None),
    }
