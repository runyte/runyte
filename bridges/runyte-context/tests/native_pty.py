# SPDX-License-Identifier: MPL-2.0
"""Launch native test editors without running Python after a multithreaded fork."""
import fcntl
import os
from pathlib import Path
import pty
import subprocess
import sys
import termios


def spawn(arguments, *, cwd, env, configure):
    master, slave = pty.openpty()
    try:
        # Geometry must exist before the editor's first frame. Popen performs
        # cwd/environment/session setup without a Python preexec_fn callback.
        configure(slave)
        process = subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), *map(str, arguments)],
            cwd=cwd, env=env, stdin=slave, stdout=slave, stderr=slave,
            close_fds=True, start_new_session=True,
        )
    except BaseException:
        os.close(master)
        raise
    finally:
        os.close(slave)
    return process, master


if __name__ == '__main__':
    # This is a fresh, single-threaded interpreter, not the forked copy of the
    # test runner and its active PTY/MCP reader threads. Become the controlling
    # terminal's session leader before replacing this process with the editor.
    try:
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        os.execv(sys.argv[1], sys.argv[1:])
    except OSError as error:
        print(f'Native PTY launch failed: {error}', file=sys.stderr)
        sys.exit(127)
