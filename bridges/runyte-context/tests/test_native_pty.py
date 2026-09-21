# SPDX-License-Identifier: MPL-2.0
"""Native PTY launch ownership, also run without a built editor."""
import fcntl
import os
from pathlib import Path
import selectors
import struct
import sys
import tempfile
import termios
import threading
import time
import unittest

from native_pty import spawn


def configure(fd):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 37, 113, 0, 0))


def output_from(process, fd):
    output = bytearray()
    deadline = time.monotonic() + 10
    with selectors.DefaultSelector() as poll:
        poll.register(fd, selectors.EVENT_READ)
        while time.monotonic() < deadline:
            if poll.select(.05):
                try:
                    data = os.read(fd, 4096)
                except OSError:
                    break  # Linux reports EIO at PTY EOF.
                if not data:
                    break
                output.extend(data)
                if len(output) > 65536:
                    raise AssertionError('PTY fixture exceeded output bound')
            elif process.poll() is not None:
                break
        else:
            raise AssertionError('PTY fixture exceeded output deadline')
    process.wait(timeout=5)
    return output.decode('utf-8', 'replace')


class NativePtyTests(unittest.TestCase):
    def test_spawn_with_active_thread_has_controlling_terminal_and_initial_geometry(self):
        stopped = threading.Event()
        thread = threading.Thread(target=stopped.wait)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as root:
                for _ in range(3):
                    process, fd = spawn(
                        [sys.executable, str(Path(__file__).resolve()), '--fixture'],
                        cwd=root, env={**os.environ, 'XDG_CONFIG_HOME': root}, configure=configure,
                    )
                    try:
                        output = output_from(process, fd)
                        self.assertEqual(process.returncode, 0, output)
                        self.assertIn('CONTROLLING_TTY:37:113', output)
                    finally:
                        if process.poll() is None:
                            process.kill()
                        process.wait(timeout=5)
                        os.close(fd)
        finally:
            stopped.set()
            thread.join(timeout=5)
        self.assertFalse(thread.is_alive())

    def test_failed_exec_exits_and_reports_the_launch_error(self):
        with tempfile.TemporaryDirectory() as root:
            process, fd = spawn(
                [str(Path(root) / 'missing-program')], cwd=root,
                env={**os.environ, 'XDG_CONFIG_HOME': root}, configure=configure,
            )
            try:
                output = output_from(process, fd)
                self.assertEqual(process.returncode, 127, output)
                self.assertIn('Native PTY launch failed:', output)
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)
                os.close(fd)


if __name__ == '__main__':
    if sys.argv[1:] == ['--fixture']:
        with open('/dev/tty', 'rb', buffering=0) as tty:
            rows, columns, _, _ = struct.unpack('HHHH', fcntl.ioctl(tty, termios.TIOCGWINSZ, b'\0' * 8))
            assert os.tcgetpgrp(tty.fileno()) == os.getpgrp()
            print(f'CONTROLLING_TTY:{rows}:{columns}', flush=True)
    else:
        unittest.main()
