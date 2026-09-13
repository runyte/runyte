# SPDX-License-Identifier: MPL-2.0
"""Run every checked-in plugin conformance suite with the current interpreter."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require-backends', action='store_true',
                        help='Require mpv, Node and built Rust/C todo examples')
    options = parser.parse_args()
    if options.require_backends:
        for variable, executable in [('MPV', 'mpv'), ('NODE', 'node')]:
            if shutil.which(os.environ.get(variable) or executable) is None:
                parser.error(f'Install {executable} or set {variable} to its executable')
    directory = Path(__file__).resolve().parent
    scripts = sorted(path for path in directory.glob('check_*.py') if path.name != 'check_all.py')
    for script in scripts:
        print(f'Running {script.name}', flush=True)
        # Each suite owns its child lifetimes and deadlines. Killing only its
        # leader cannot clean up fixtures that intentionally test new groups.
        # Let that ownership unwind; CI bounds the whole job separately.
        arguments = ['--require-all'] if options.require_backends and script.name == 'check_todo.py' else []
        result = subprocess.run([sys.executable, str(script)] + arguments)
        if result.returncode:
            return 1
    print(f'Passed all {len(scripts)} plugin conformance suites', flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
