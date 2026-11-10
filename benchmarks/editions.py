#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

"""Readiness, quit and idle across Runyte's editions and modes.

Every cell measures the same contract as `startup.py`: from before the launch
until a one-character edit is visible in a completed frame, with a later
whole-file save proving it was a real edit. See README.md for the desktop
display, the pixel comparison and what each mode's columns contain.
"""

from __future__ import annotations

import argparse
import ctypes as C
import hashlib
import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from datetime import date
from pathlib import Path

import fixtures
import ptybench
import run as baseline
import startup

EDITIONS = ("terminal", "desktop")
# Mux takes a workspace rather than a file, so a cold launch reaches the
# document through `:open`; reattaching finds it already open in the host.
MODES = ("editor", "ide", "mux-cold", "mux-warm")
IDLE_MODES = ("editor", "ide", "mux")
TIMEOUT = 40.0
ABOUT_TITLE = "[about]"
# A window that fails to detach stays open on its error page; do not wait the
# full quit budget to learn that.
DETACH_TIMEOUT_SECONDS = 3.0
OPEN_SETTLE_SECONDS = 0.4

WINDOW_NAME = b"Runyte"
# The default window is 120x40 cells of 9x20 pixels, matching the PTY geometry.
WINDOW_SIZE = (1080, 800)
CELL_WIDTH, CELL_HEIGHT = 9, 20
# The caret occupies the first cells of the edited row before and after the
# edit, so comparison starts past it and spans at most this many cells.
CARET_CELLS = 3
BAND_CELLS = 30
SHIFT_MASK = 1
XKB_USE_CORE_KEYBOARD = 0x100
ALL_PLANES = 0xffffffffffffffff
Z_PIXMAP = 2
KEY_PRESS, KEY_RELEASE = 2, 3
KEY_PRESS_MASK, KEY_RELEASE_MASK = 1, 2
# Green channel above this is glyph ink on the default dark background.
INK_LEVEL = 48
INK_TABLE = bytes(1 if value > INK_LEVEL else 0 for value in range(256))
# Syntax colors arrive after the first frame and move antialiased edges across
# the threshold; a frame is the same text when its shape is this close.
MASK_TOLERANCE = 0.06
MINIMUM_TOLERANCE = 8


def launch_arguments(edition: str, mode: str, name: str) -> list[str]:
    """Options and target after the binary path for one edition and mode."""
    window = ["--window"] if edition == "desktop" else []
    if mode.startswith("mux"):
        return window + ["--mux"]
    return window + [f"--{mode}", name]


def mask(pixels: bytes) -> bytes:
    """One byte per 32-bit pixel: whether it holds glyph ink."""
    return pixels[1::4].translate(INK_TABLE)


def same_shape(left: bytes, right: bytes) -> bool:
    if len(left) != len(right):
        return False
    ink = max(sum(left), sum(right))
    if not ink:
        return False
    different = (int.from_bytes(left, "little") ^ int.from_bytes(right, "little")).bit_count()
    return different <= max(MINIMUM_TOLERANCE, ink * MASK_TOLERANCE)


def edited_band(before: bytes, after: bytes, stride: int, height: int):
    """Region of the row a leading-space edit changed, past the caret cells."""
    row_bytes = CELL_HEIGHT * stride
    rows = [
        sum(a != b for a, b in zip(before[top:top + row_bytes], after[top:top + row_bytes]))
        for top in range(0, height * stride, row_bytes)
    ]
    if not any(rows):
        raise ValueError("the edit did not change the window")
    top = rows.index(max(rows)) * CELL_HEIGHT
    columns = []
    for line in range(top, top + CELL_HEIGHT):
        start = line * stride
        columns += [
            offset // 4
            for offset in range(0, stride, 4)
            if before[start + offset:start + offset + 4] != after[start + offset:start + offset + 4]
        ]
    left = min(columns) + CARET_CELLS * CELL_WIDTH
    right = min(max(columns) + 1, left + BAND_CELLS * CELL_WIDTH)
    if right <= left:
        raise ValueError("the edited row is too short to compare")
    return left, top, right - left, CELL_HEIGHT


def quit_cell(samples: list[dict], runs: int) -> str:
    """Quit summary that reports failed detaches rather than dropping them."""
    values = [sample["quit"] for sample in samples if "quit" in sample]
    failures = sum(1 for sample in samples if sample.get("detach_failed"))
    if failures and len(values) + failures == runs and values:
        return f"{statistics.median(values):.1f} ({min(values):.1f}–{max(values):.1f}); {failures}/{runs} detaches failed"
    if failures and not values:
        return f"{failures}/{runs} detaches failed"
    return summary(values, runs)


def summary(values: list[float], runs: int, digits: int = 1) -> str:
    if len(values) != runs:
        return f"incomplete ({len(values)}/{runs})"
    return f"{statistics.median(values):.{digits}f} ({min(values):.{digits}f}–{max(values):.{digits}f})"


def busy_cores(interval: float = 1.0) -> float:
    """Logical CPUs in use across the machine over `interval` seconds."""
    def counters():
        fields = [int(field) for field in Path("/proc/stat").read_text().splitlines()[0].split()[1:]]
        return sum(fields), fields[3] + fields[4]

    total_before, idle_before = counters()
    time.sleep(interval)
    total_after, idle_after = counters()
    total = total_after - total_before
    busy_fraction = 1 - (idle_after - idle_before) / total if total else 0.0
    return busy_fraction * (os.cpu_count() or 1)


def wait_until_quiet(limit: float, timeout: float) -> dict:
    """Hold a round while other work is running; a build dominates these timings."""
    started = time.monotonic()
    while True:
        busy = busy_cores()
        waited = time.monotonic() - started
        if busy <= limit:
            return {"busy_cores": round(busy, 2), "waited_seconds": round(waited, 1), "quiet": True}
        if waited >= timeout:
            return {"busy_cores": round(busy, 2), "waited_seconds": round(waited, 1), "quiet": False}


def stop_host(binary: str, env: dict[str, str], directory: Path) -> None:
    environment = ptybench.clean_environment()
    environment.update(env)
    subprocess.run(
        [binary, "--session-stop", "--force", str(directory)],
        env=environment, cwd=directory, stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL, timeout=20, check=False,
    )


def host_pids(directory: Path) -> set[int]:
    """Persistent hosts are detached, so the client's process tree omits them."""
    pids = set()
    for root in (baseline.EMPTY_STATE, directory):
        for path in root.rglob("endpoint.json"):
            try:
                pid = json.loads(path.read_text()).get("pid")
            except (OSError, ValueError):
                continue
            if isinstance(pid, int) and Path(f"/proc/{pid}").exists():
                pids.add(pid)
    return pids


def session_ticks(pid: int, hosts: set[int]) -> int:
    return ptybench._cpu_ticks(pid) + sum(ptybench._cpu_ticks(host) for host in hosts if host != pid)


# Terminal edition


DETACH_SEQUENCE = ((b"\x1b", 0.08), (b":", 0.05), (b"detach", 0.05), (b"\r", 0.0))


def terminal_exit(pid, fd, terminal, sequence) -> float:
    """Milliseconds from the final key of `sequence` to a successful exit."""
    for part, gap in sequence[:-1]:
        startup.keys(fd, terminal, ((part, gap),))
    os.write(fd, sequence[-1][0])
    started = time.perf_counter()
    deadline = started + ptybench.QUIT_TIMEOUT_SECONDS
    while time.perf_counter() < deadline:
        exited, status = os.waitpid(pid, os.WNOHANG)
        if exited:
            if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
                raise RuntimeError("editor exited unsuccessfully")
            return (time.perf_counter() - started) * 1000
        try:
            startup.pump(fd, terminal, 0.001)
        except (EOFError, OSError):
            time.sleep(0.0005)
    raise TimeoutError("editor did not exit")


def terminal_open(fd, terminal, name, deadline):
    startup.until(fd, terminal, lambda: terminal.contains(ABOUT_TITLE), deadline, "about pane")
    os.write(fd, f":open {name}\r".encode())


def retain_terminal_document(binary, env, fixture):
    """Start a host holding the fixture, then leave it running detached."""
    terminal = startup.Terminal()
    deadline = time.perf_counter() + TIMEOUT
    pid, fd = ptybench._spawn([binary, "--mux"], env, str(fixture.parent))
    try:
        terminal_open(fd, terminal, fixture.name, deadline)
        startup.until(fd, terminal, lambda: terminal.contains(startup.DOCUMENT_TEXT),
                      deadline, "retained document display")
        terminal_exit(pid, fd, terminal, DETACH_SEQUENCE)
    finally:
        ptybench._reap(pid)
        os.close(fd)


def terminal_sample(binary, env, fixture, mode):
    original = fixture.read_bytes()
    mux = mode.startswith("mux")
    with tempfile.TemporaryDirectory(prefix="runyte-editions-") as directory:
        saved = Path(directory) / "verified.txt"
        try:
            if mode == "mux-warm":
                retain_terminal_document(binary, env, fixture)
            terminal = startup.Terminal()
            start = time.perf_counter()
            deadline = start + TIMEOUT
            argv = [binary] + launch_arguments("terminal", mode, fixture.name)
            pid, fd = ptybench._spawn(argv, env, str(fixture.parent))
            try:
                if mode == "mux-cold":
                    terminal_open(fd, terminal, fixture.name, deadline)
                startup.until(fd, terminal, lambda: terminal.contains(startup.DOCUMENT_TEXT),
                              deadline, "document display")
                visible = (time.perf_counter() - start) * 1000
                position = terminal.position(startup.DOCUMENT_TEXT)
                os.write(fd, b"i" + startup.EDIT)
                edited = startup.EDIT.decode() + startup.DOCUMENT_TEXT
                startup.until(fd, terminal, lambda: terminal.at(position, edited),
                              deadline, "edited document display")
                ready = (time.perf_counter() - start) * 1000
                startup.keys(fd, terminal, ((b"\x1b", 0.08), (f":write! {saved}\r".encode(), 0.0)))

                def saved_correctly():
                    try:
                        return saved.read_bytes() == startup.EDIT + original
                    except OSError:
                        return False

                startup.until(fd, terminal, saved_correctly, deadline, "saved edit verification")
                quit_ms = terminal_exit(pid, fd, terminal,
                                        DETACH_SEQUENCE if mux else ptybench.QUIT_SEQUENCE)
                return {"document_visible": visible, "ready_to_edit": ready, "quit": quit_ms}
            finally:
                ptybench._reap(pid)
                os.close(fd)
        finally:
            if mux:
                stop_host(binary, env, fixture.parent)


def terminal_idle(binary, env, fixture, mode, settle, window):
    deadline = time.perf_counter() + TIMEOUT
    mux = mode == "mux"
    terminal = startup.Terminal()
    argv = [binary] + launch_arguments("terminal", mode, fixture.name)
    pid, fd = ptybench._spawn(argv, env, str(fixture.parent))
    try:
        if mux:
            terminal_open(fd, terminal, fixture.name, deadline)
        startup.until(fd, terminal, lambda: terminal.contains(startup.DOCUMENT_TEXT),
                      deadline, "document display")
        settled = time.perf_counter() + settle
        while time.perf_counter() < settled:
            startup.pump(fd, terminal, 0.05)
        hosts = host_pids(fixture.parent) if mux else set()
        before = session_ticks(pid, hosts)
        started = time.perf_counter()
        writes = 0
        while time.perf_counter() - started < window:
            size = terminal_drain(fd, terminal, 0.05)
            writes += 1 if size else 0
        elapsed = time.perf_counter() - started
        ticks = session_ticks(pid, hosts) - before
        return {"cpu_percent": ticks / os.sysconf("SC_CLK_TCK") / elapsed * 100.0, "writes": writes}
    finally:
        ptybench._reap(pid)
        os.close(fd)
        if mux:
            stop_host(binary, env, fixture.parent)


def terminal_drain(fd, terminal, timeout):
    import select
    if not select.select([fd], [], [], timeout)[0]:
        return 0
    data = os.read(fd, 65536)
    if not data:
        raise EOFError("editor closed the terminal")
    reply = terminal.feed(data)
    if reply:
        os.write(fd, reply)
    return len(data)


# Desktop edition


class XImage(C.Structure):
    _fields_ = [("width", C.c_int), ("height", C.c_int), ("xoffset", C.c_int), ("format", C.c_int),
                ("data", C.c_void_p), ("byte_order", C.c_int), ("bitmap_unit", C.c_int),
                ("bitmap_bit_order", C.c_int), ("bitmap_pad", C.c_int), ("depth", C.c_int),
                ("bytes_per_line", C.c_int), ("bits_per_pixel", C.c_int)]


class XKeyEvent(C.Structure):
    _fields_ = [("type", C.c_int), ("serial", C.c_ulong), ("send_event", C.c_int),
                ("display", C.c_void_p), ("window", C.c_ulong), ("root", C.c_ulong),
                ("subwindow", C.c_ulong), ("time", C.c_ulong), ("x", C.c_int), ("y", C.c_int),
                ("x_root", C.c_int), ("y_root", C.c_int), ("state", C.c_uint), ("keycode", C.c_uint),
                ("same_screen", C.c_int), ("padding", C.c_long * 12)]


KEY_NAMES = {":": ("semicolon", True), "!": ("1", True), "/": ("slash", False), ".": ("period", False),
             "-": ("minus", False), "_": ("minus", True), " ": ("space", False)}


def key_name(character: str) -> tuple[str, bool]:
    return KEY_NAMES.get(character, (character.lower(), character.isupper()))


@C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
def ignore_x_error(_display, _event):
    # A window that disappears mid-request is an ordinary end of a sample.
    return 0


class Display:
    """An isolated virtual compositor with Xwayland, reached over its own socket.

    It never maps a window on the person's desktop. Keys are delivered with
    XSendEvent because KWin routes XTEST through its input-capture portal,
    which needs interactive consent that a headless compositor cannot give.
    """

    def __init__(self, env=None):
        self.runtime = tempfile.mkdtemp(prefix="rb")
        known = set(os.listdir("/tmp/.X11-unix"))
        environment = ptybench.clean_environment()
        # The compositor and its helpers need fixture-owned storage too, not
        # just the editor windows launched inside it.
        if env is None:
            from session_navigation import environment as isolated_environment
            env = isolated_environment(Path(self.runtime))
        environment.update(env)
        for name in ("WAYLAND_DISPLAY", "DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "XAUTHORITY"):
            environment.pop(name, None)
        environment["XDG_RUNTIME_DIR"] = self.runtime
        width, height = WINDOW_SIZE
        self.log = open(Path(self.runtime) / "kwin.log", "w")
        self.compositor = None
        try:
            self.compositor = subprocess.Popen(
                ["dbus-run-session", "--", "kwin_wayland", "--virtual", "--xwayland", "--socket", "rb",
                 "--width", str(width + 320), "--height", str(height + 200),
                 "--no-lockscreen", "--no-global-shortcuts", "--no-kactivities"],
                env=environment, stdout=self.log, stderr=self.log, start_new_session=True,
            )
            self.number = self.wait_for_socket(known)
            self.x = C.CDLL("libX11.so.6")
            self.damage = C.CDLL("libXdamage.so.1")
            self.declare()
            self.x.XSetErrorHandler(C.cast(ignore_x_error, C.c_void_p))
            self.connection = self.connect()
            self.root = self.x.XDefaultRootWindow(self.connection)
            self.event_base = C.c_int()
            error_base = C.c_int()
            if not self.damage.XDamageQueryExtension(self.connection, C.byref(self.event_base), C.byref(error_base)):
                raise RuntimeError("the display has no Damage extension")
        except BaseException:
            self.close()
            raise

    def declare(self):
        x, damage = self.x, self.damage
        x.XOpenDisplay.argtypes = [C.c_char_p]
        x.XOpenDisplay.restype = C.c_void_p
        x.XDefaultRootWindow.argtypes = [C.c_void_p]
        x.XDefaultRootWindow.restype = C.c_ulong
        x.XSetErrorHandler.argtypes = [C.c_void_p]
        x.XFlush.argtypes = [C.c_void_p]
        x.XSync.argtypes = [C.c_void_p, C.c_int]
        x.XFree.argtypes = [C.c_void_p]
        x.XPending.argtypes = [C.c_void_p]
        x.XNextEvent.argtypes = [C.c_void_p, C.c_void_p]
        x.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong),
                                 C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
        x.XFetchName.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_char_p)]
        x.XGetGeometry.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_int),
                                   C.POINTER(C.c_int), C.POINTER(C.c_uint), C.POINTER(C.c_uint),
                                   C.POINTER(C.c_uint), C.POINTER(C.c_uint)]
        x.XGetImage.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_int, C.c_uint, C.c_uint, C.c_ulong, C.c_int]
        x.XGetImage.restype = C.c_void_p
        x.XDestroyImage.argtypes = [C.c_void_p]
        x.XStringToKeysym.argtypes = [C.c_char_p]
        x.XStringToKeysym.restype = C.c_ulong
        x.XKeysymToKeycode.argtypes = [C.c_void_p, C.c_ulong]
        x.XKeysymToKeycode.restype = C.c_uint
        x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.c_void_p]
        x.XkbLockModifiers.argtypes = [C.c_void_p, C.c_uint, C.c_uint, C.c_uint]
        damage.XDamageQueryExtension.argtypes = [C.c_void_p, C.POINTER(C.c_int), C.POINTER(C.c_int)]
        damage.XDamageCreate.argtypes = [C.c_void_p, C.c_ulong, C.c_int]
        damage.XDamageCreate.restype = C.c_ulong
        damage.XDamageSubtract.argtypes = [C.c_void_p, C.c_ulong, C.c_ulong, C.c_ulong]
        damage.XDamageDestroy.argtypes = [C.c_void_p, C.c_ulong]

    def wait_for_socket(self, known):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self.compositor.poll() is not None:
                diagnostic = (Path(self.runtime) / "kwin.log").read_text(errors="replace")[-4000:]
                raise RuntimeError(f"the virtual compositor exited during startup:\n{diagnostic}")
            created = set(os.listdir("/tmp/.X11-unix")) - known
            if created and (Path(self.runtime) / "rb").exists():
                return min(int(name[1:]) for name in created)
            time.sleep(0.05)
        raise TimeoutError("the virtual compositor did not create an X display")

    def connect(self):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            connection = self.x.XOpenDisplay(f":{self.number}".encode())
            if connection:
                return connection
            time.sleep(0.1)
        raise TimeoutError("could not connect to the virtual X display")

    def environment(self, env):
        environment = ptybench.clean_environment()
        for name in list(environment):
            if name.startswith("RUNYTE_"):
                environment.pop(name)
        environment.update(env)
        for name in ("WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "XAUTHORITY"):
            environment.pop(name, None)
        environment["DISPLAY"] = f":{self.number}"
        environment["SHELL"] = "/bin/sh"
        # A measured window must not reach the person's real session bus.
        environment["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={self.runtime}/no-session-bus"
        return environment

    def children(self):
        root, parent = C.c_ulong(), C.c_ulong()
        windows = C.POINTER(C.c_ulong)()
        count = C.c_uint()
        self.x.XQueryTree(self.connection, self.root, C.byref(root), C.byref(parent), C.byref(windows), C.byref(count))
        values = [windows[index] for index in range(count.value)]
        if windows:
            self.x.XFree(windows)
        return values

    def name(self, window):
        name = C.c_char_p()
        self.x.XFetchName(self.connection, window, C.byref(name))
        value = name.value
        if name:
            self.x.XFree(name)
        return value

    def size(self, window):
        root = C.c_ulong()
        x, y = C.c_int(), C.c_int()
        width, height, border, depth = C.c_uint(), C.c_uint(), C.c_uint(), C.c_uint()
        status = self.x.XGetGeometry(self.connection, window, C.byref(root), C.byref(x), C.byref(y),
                                     C.byref(width), C.byref(height), C.byref(border), C.byref(depth))
        return (width.value, height.value) if status else None

    def find_window(self):
        for window in self.children():
            if (self.name(window) or b"").startswith(WINDOW_NAME) and self.size(window) == WINDOW_SIZE:
                return window
        return None

    def capture(self, window, x, y, width, height):
        """Raw 32-bit pixels and row stride, or None while the window is unready."""
        image = self.x.XGetImage(self.connection, window, x, y, width, height, ALL_PLANES, Z_PIXMAP)
        if not image:
            return None
        header = XImage.from_address(image)
        stride = header.bytes_per_line
        data = C.string_at(header.data, stride * header.height)
        self.x.XDestroyImage(image)
        return data, stride

    def key(self, window, name, shift=False):
        keycode = self.x.XKeysymToKeycode(self.connection, self.x.XStringToKeysym(name.encode()))
        if not keycode:
            raise ValueError(f"no keycode for {name}")
        # The window reads modifiers from XKB state notifications, which a sent
        # event's own state field does not produce.
        if shift:
            self.x.XkbLockModifiers(self.connection, XKB_USE_CORE_KEYBOARD, SHIFT_MASK, SHIFT_MASK)
        for kind, event_mask in ((KEY_PRESS, KEY_PRESS_MASK), (KEY_RELEASE, KEY_RELEASE_MASK)):
            event = XKeyEvent(type=kind, display=self.connection, window=window, root=self.root,
                              keycode=keycode, state=SHIFT_MASK if shift else 0, same_screen=1)
            self.x.XSendEvent(self.connection, window, 1, event_mask, C.byref(event))
        if shift:
            self.x.XkbLockModifiers(self.connection, XKB_USE_CORE_KEYBOARD, SHIFT_MASK, 0)

    def text(self, window, text):
        for character in text:
            self.key(window, *key_name(character))

    def flush(self):
        self.x.XFlush(self.connection)

    def close(self):
        if getattr(self, "compositor", None) and self.compositor.poll() is None:
            os.killpg(self.compositor.pid, signal.SIGTERM)
            try:
                self.compositor.wait(10)
            except subprocess.TimeoutExpired:
                os.killpg(self.compositor.pid, signal.SIGKILL)
                self.compositor.wait(10)
        self.log.close()
        shutil.rmtree(self.runtime, ignore_errors=True)


def poll(predicate, deadline, observation, process=None):
    while True:
        value = predicate()
        if value:
            return value
        if process is not None and process.poll() is not None:
            raise RuntimeError(f"editor exited {process.returncode} before {observation}")
        if time.perf_counter() >= deadline:
            raise TimeoutError(f"editor did not complete {observation}")
        time.sleep(0.0005)


def stable_frame(display, window, deadline, process, quiet=OPEN_SETTLE_SECONDS, reject=None):
    """A whole-window capture that stayed unchanged for `quiet` seconds."""
    width, height = WINDOW_SIZE
    frame = None
    changed = time.perf_counter()
    while True:
        captured = display.capture(window, 0, 0, width, height)
        if captured and captured[0] != (frame[0] if frame else None):
            frame, changed = captured, time.perf_counter()
        elif (frame and len(set(frame[0][1::4])) > 8
              and (reject is None or not reject(frame[0]))
              and time.perf_counter() - changed >= quiet):
            return frame
        if process.poll() is not None:
            raise RuntimeError(f"editor exited {process.returncode} before settling")
        if time.perf_counter() >= deadline:
            raise TimeoutError("window did not settle")
        time.sleep(0.01)


class DesktopSession:
    """One window process, with its isolated stderr retained for failures."""

    def __init__(self, display, binary, env, fixture, mode):
        self.display = display
        self.directory = tempfile.TemporaryDirectory(prefix="runyte-editions-")
        self.root = Path(self.directory.name)
        self.log = open(self.root / "window.log", "w")
        self.started = time.perf_counter()
        self.deadline = self.started + TIMEOUT
        argv = [binary] + launch_arguments("desktop", mode, fixture.name)
        self.process = None
        try:
            self.process = subprocess.Popen(argv, cwd=fixture.parent, env=display.environment(env),
                                            stdout=self.log, stderr=self.log)
            self.window = poll(display.find_window, self.deadline, "window creation", self.process)
        except BaseException as error:
            self.log.flush()
            details = (self.root / "window.log").read_text(errors="replace")[-8000:]
            self.close()
            if isinstance(error, Exception):
                raise RuntimeError(f"{error}\nWindow log:\n{details}") from error
            raise
        self.mapped = (time.perf_counter() - self.started) * 1000

    def key(self, name):
        self.display.key(self.window, name)
        self.display.flush()

    def command(self, text):
        self.display.text(self.window, ":" + text)
        self.display.key(self.window, "Return")
        self.display.flush()

    def elapsed(self):
        return (time.perf_counter() - self.started) * 1000

    def wait_for_band(self, band, reference, observation):
        def matches():
            captured = self.display.capture(self.window, *band)
            return captured is not None and same_shape(mask(captured[0]), reference)

        poll(matches, self.deadline, observation, self.process)

    def exit_after(self, send, timeout) -> float:
        send()
        started = time.perf_counter()
        # Popen.wait backs off to tens of milliseconds, which would be reported
        # as the exit time.
        deadline = started + timeout
        while self.process.poll() is None:
            if time.perf_counter() >= deadline:
                raise TimeoutError("editor did not exit")
            time.sleep(0.0002)
        elapsed = (time.perf_counter() - started) * 1000
        if self.process.returncode != 0:
            raise RuntimeError(f"editor exited {self.process.returncode}")
        return elapsed

    def close(self):
        if self.process is not None and self.process.poll() is None:
            self.process.kill()
            self.process.wait(10)
        self.log.close()
        self.directory.cleanup()


def desktop_exit(session, mux):
    """Exit milliseconds, or None when a mux window failed to detach."""
    session.key("Escape")
    time.sleep(0.08)

    def send():
        session.command("detach" if mux else "q!")

    try:
        return session.exit_after(send, DETACH_TIMEOUT_SECONDS if mux else ptybench.QUIT_TIMEOUT_SECONDS)
    except TimeoutError:
        if mux:
            return None
        raise


def retain_desktop_document(display, binary, env, fixture, about):
    session = DesktopSession(display, binary, env, fixture, "mux-cold")
    try:
        session.wait_for_band(about[0], about[1], "about pane")
        session.command(f"open {fixture.name}")
        stable_frame(display, session.window, session.deadline, session.process,
                     reject=lambda frame: same_shape(mask(region(frame, about[0])), about[1]))
        desktop_exit(session, True)
    finally:
        session.close()


def region(frame, band):
    x, y, width, height = band
    stride = WINDOW_SIZE[0] * 4
    return b"".join(frame[top * stride + x * 4:top * stride + (x + width) * 4] for top in range(y, y + height))


def desktop_calibration(display, binary, env, fixture, mode, about):
    """Learn, in the discarded launch, the pixels of the document and its edit.

    Doubles as the warm-up. Later samples compare only this one row against
    these references; they neither wait for the window to settle nor sleep.
    """
    mux = mode.startswith("mux")
    if mode == "mux-warm":
        retain_desktop_document(display, binary, env, fixture, about)
    session = DesktopSession(display, binary, env, fixture, mode)
    try:
        reject = None
        if mode == "mux-cold":
            first = stable_frame(display, session.window, session.deadline, session.process)
            about_band = (0, 0, 300, CELL_HEIGHT)
            about = (about_band, mask(region(first[0], about_band)))
            session.command(f"open {fixture.name}")
            reject = lambda frame: same_shape(mask(region(frame, about_band)), about[1])
        before = stable_frame(display, session.window, session.deadline, session.process, reject=reject)
        session.key("i")
        session.key("space")
        after = stable_frame(display, session.window, session.deadline, session.process,
                             reject=lambda frame: frame == before[0])
        band = edited_band(before[0], after[0], before[1], WINDOW_SIZE[1])
        references = (mask(region(before[0], band)), mask(region(after[0], band)))
        if same_shape(*references):
            raise ValueError("document and edit references are indistinguishable")
        desktop_exit(session, mux)
        return {"band": band, "before": references[0], "after": references[1], "about": about}
    finally:
        session.close()
        if mux:
            stop_host(binary, env, fixture.parent)


def desktop_sample(display, binary, env, fixture, mode, calibration):
    original = fixture.read_bytes()
    mux = mode.startswith("mux")
    band, before, after = calibration["band"], calibration["before"], calibration["after"]
    try:
        if mode == "mux-warm":
            retain_desktop_document(display, binary, env, fixture, calibration["about"])
        session = DesktopSession(display, binary, env, fixture, mode)
        try:
            if mode == "mux-cold":
                session.wait_for_band(*calibration["about"], "about pane")
                session.command(f"open {fixture.name}")
            session.wait_for_band(band, before, "document display")
            visible = session.elapsed()
            session.key("i")
            session.key("space")
            session.wait_for_band(band, after, "edited document display")
            ready = session.elapsed()
            saved = session.root / "verified.txt"
            session.key("Escape")
            time.sleep(0.08)
            session.command(f"write! {saved}")

            def saved_correctly():
                try:
                    return saved.read_bytes() == startup.EDIT + original
                except OSError:
                    return False

            poll(saved_correctly, session.deadline, "saved edit verification", session.process)
            quit_ms = desktop_exit(session, mux)
            result = {"window_mapped": session.mapped, "document_visible": visible, "ready_to_edit": ready}
            result.update({"quit": quit_ms} if quit_ms is not None else {"detach_failed": True})
            return result
        finally:
            session.close()
    finally:
        if mux:
            stop_host(binary, env, fixture.parent)


def desktop_idle(display, binary, env, fixture, mode, settle, window, calibration):
    mux = mode == "mux"
    x, connection = display.x, display.connection
    session = DesktopSession(display, binary, env, fixture, "mux-cold" if mux else mode)
    damage = None
    try:
        if mux:
            session.wait_for_band(*calibration["about"], "about pane")
            session.command(f"open {fixture.name}")
        session.wait_for_band(calibration["band"], calibration["before"], "document display")
        time.sleep(settle)
        hosts = host_pids(fixture.parent) if mux else set()
        damage = display.damage.XDamageCreate(connection, session.window, 3)
        x.XSync(connection, 0)
        event = C.create_string_buffer(256)
        while x.XPending(connection):
            x.XNextEvent(connection, event)
        display.damage.XDamageSubtract(connection, damage, 0, 0)
        x.XSync(connection, 0)
        before = session_ticks(session.process.pid, hosts)
        started = time.perf_counter()
        repaints = 0
        while time.perf_counter() - started < window:
            if session.process.poll() is not None:
                raise RuntimeError("editor exited during idle measurement")
            while x.XPending(connection):
                x.XNextEvent(connection, event)
                if C.c_int.from_buffer(event).value == display.event_base.value:
                    repaints += 1
                    display.damage.XDamageSubtract(connection, damage, 0, 0)
            x.XFlush(connection)
            time.sleep(0.02)
        elapsed = time.perf_counter() - started
        ticks = session_ticks(session.process.pid, hosts) - before
        return {"cpu_percent": ticks / os.sysconf("SC_CLK_TCK") / elapsed * 100.0, "writes": repaints}
    finally:
        if damage:
            display.damage.XDamageDestroy(connection, damage)
            x.XFlush(connection)
        session.close()
        if mux:
            stop_host(binary, env, fixture.parent)


# Report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=baseline.positive_run_count, default=10)
    parser.add_argument("--idle-runs", type=baseline.idle_run_count, default=5)
    parser.add_argument("--idle-window", type=float, default=16.0,
                        help="seconds per idle window; spans the 15-second session observation")
    parser.add_argument("--idle-settle", type=float, default=2.5)
    parser.add_argument("--no-idle", action="store_true")
    parser.add_argument("--fixtures", default=",".join(fixtures.FIXTURES))
    parser.add_argument("--editions", default=",".join(EDITIONS))
    parser.add_argument("--terminal-binary", type=Path, default=baseline.REPO / "target/release/runyte")
    parser.add_argument("--desktop-binary", type=Path, default=baseline.REPO / "target/release/runyte-desktop")
    parser.add_argument("--json", type=Path, help="retain all samples and binary hashes")
    parser.add_argument("--max-busy-cores", type=float, default=1.0,
                        help="hold each round until no more than this many logical CPUs are busy")
    parser.add_argument("--quiet-timeout", type=float, default=3600.0,
                        help="seconds to hold a round before measuring anyway and recording that")
    args = parser.parse_args()
    editions = args.editions.split(",")
    names = args.fixtures.split(",")
    for name in names:
        fixtures.split(name)
    paths = baseline.prepare()
    env = baseline.environment()
    binaries = {"terminal": str(args.terminal_binary.resolve()),
                "desktop": str(args.desktop_binary.resolve())}
    info = {}
    for edition in editions:
        path = Path(binaries[edition])
        if not path.exists():
            parser.error(f"missing {edition} binary; build it with cargo build --release --locked")
        info[edition] = {"version": baseline.version_of([str(path)], env, str(baseline.FIXTURES)),
                         "bytes": path.stat().st_size,
                         "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    display = Display() if "desktop" in editions else None
    records = []
    idle_records = []
    calibrations = {}
    quiet_log = []
    failed = False

    def hold(label):
        state = wait_until_quiet(args.max_busy_cores, args.quiet_timeout)
        quiet_log.append({"round": label, **state})
        if state["waited_seconds"] >= 2 or not state["quiet"]:
            print(f"held {label}: {state}", file=sys.stderr, flush=True)

    def record(target, base, measure):
        nonlocal failed
        try:
            sample = measure()
        except (OSError, EOFError, TimeoutError, ValueError, RuntimeError,
                subprocess.SubprocessError) as error:
            print(f"{base}: {error}", file=sys.stderr, flush=True)
            sample = {}
            failed = True
        if base.pop("keep", True):
            target.append({**base, **sample})
        return sample

    try:
        for fixture_name in names:
            fixture = paths[fixture_name]
            if display:
                hold(f"calibrate {fixture_name}")
                for mode in MODES:
                    about = None
                    if mode == "mux-warm":
                        about = calibrations[fixture_name, "mux-cold"]["about"]
                    calibrations[fixture_name, mode] = desktop_calibration(
                        display, binaries["desktop"], env, fixture, mode, about)
                    print(f"calibrated {fixture_name} {mode}", file=sys.stderr, flush=True)
            cells = [(edition, mode) for edition in editions for mode in MODES]
            for round_index in range(args.runs + 1):
                # The desktop calibration launches serve as that cell's warm-up.
                keep = bool(round_index)
                hold(f"{fixture_name} round {round_index}")
                for offset in range(len(cells)):
                    edition, mode = cells[(offset + round_index) % len(cells)]
                    if edition == "desktop" and not round_index:
                        continue
                    base = dict(edition=edition, mode=mode, fixture=fixture_name,
                                round=round_index, keep=keep)
                    if edition == "terminal":
                        record(records, base,
                               lambda: terminal_sample(binaries["terminal"], env, fixture, mode))
                    else:
                        calibration = calibrations[fixture_name, mode]
                        record(records, base,
                               lambda: desktop_sample(display, binaries["desktop"], env, fixture, mode, calibration))
            print(f"measured {fixture_name}", file=sys.stderr, flush=True)
        if not args.no_idle:
            fixture = paths[baseline.IDLE_FIXTURE]
            idle_calibrations = {}
            if display:
                for mode in ("editor", "ide", "mux-cold"):
                    idle_calibrations[mode] = calibrations.get((baseline.IDLE_FIXTURE, mode)) or (
                        desktop_calibration(display, binaries["desktop"], env, fixture, mode, None))
            for round_index in range(args.idle_runs):
                hold(f"idle round {round_index + 1}")
                for edition in editions:
                    for mode in IDLE_MODES:
                        base = dict(edition=edition, mode=mode, round=round_index + 1, keep=True)
                        if edition == "terminal":
                            record(idle_records, base, lambda: terminal_idle(
                                binaries["terminal"], env, fixture, mode, args.idle_settle, args.idle_window))
                        else:
                            calibration = idle_calibrations["mux-cold" if mode == "mux" else mode]
                            record(idle_records, base, lambda: desktop_idle(
                                display, binaries["desktop"], env, fixture, mode, args.idle_settle,
                                args.idle_window, calibration))
                print(f"idle round {round_index + 1}", file=sys.stderr, flush=True)
    finally:
        if display:
            display.close()
    if args.json:
        artifact = {
            "date": date.today().isoformat(),
            "machine": subprocess.check_output(["uname", "-srmo"], text=True).strip(),
            "binaries": {edition: info[edition] for edition in editions},
            "fixtures": names,
            "runs": args.runs,
            "idle_runs": args.idle_runs,
            "idle_window_seconds": args.idle_window,
            "quiet": quiet_log,
            "samples": records,
            "idle": idle_records,
        }
        args.json.write_text(json.dumps(artifact, indent=2) + "\n")
    print(f"## {date.today().isoformat()} — editions and modes\n")
    for edition in editions:
        print(f"- {edition}: `{info[edition]['version']}`, {info[edition]['bytes']:,} bytes, "
              f"SHA-256 `{info[edition]['sha256']}`")
    print()

    def values(edition, mode, fixture_name, metric):
        return [sample[metric] for sample in records
                if sample["edition"] == edition and sample["mode"] == mode
                and sample["fixture"] == fixture_name and metric in sample]

    for metric, title in (("ready_to_edit", "Ready to edit"), ("document_visible", "Document visible"),
                          ("quit", "Quit (detach for mux)")):
        print(f"### {title}, median (min–max) ms\n")
        columns = [(edition, mode) for edition in editions for mode in MODES]
        print("| Fixture | " + " | ".join(f"{edition} {mode}" for edition, mode in columns) + " |")
        print("| --- | " + " | ".join("---:" for _ in columns) + " |")
        for fixture_name in names:
            if metric == "quit":
                cells = [quit_cell([sample for sample in records if sample["edition"] == edition
                                    and sample["mode"] == mode and sample["fixture"] == fixture_name],
                                   args.runs) for edition, mode in columns]
            else:
                cells = [summary(values(edition, mode, fixture_name, metric), args.runs)
                         for edition, mode in columns]
            print(f"| `{fixture_name}` | " + " | ".join(cells) + " |")
        print()
    if not args.no_idle:
        print(f"### Idle, `{baseline.IDLE_FIXTURE}`, {args.idle_runs} windows of {args.idle_window:g} s\n")
        print("| Edition | Mode | CPU %, median (range) | Screen writes or repaints, median (range) |")
        print("| --- | --- | ---: | ---: |")
        for edition in editions:
            for mode in IDLE_MODES:
                rows = [sample for sample in idle_records
                        if sample["edition"] == edition and sample["mode"] == mode]
                cpu = [sample["cpu_percent"] for sample in rows if "cpu_percent" in sample]
                writes = [sample["writes"] for sample in rows if "writes" in sample]
                print(f"| {edition} | {mode} | {summary(cpu, args.idle_runs, 2)} | "
                      f"{summary(writes, args.idle_runs, 0)} |")
        print()
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
