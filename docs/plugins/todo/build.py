# SPDX-License-Identifier: MPL-2.0
"""Build the compiled todo variants and generate an opt-in demo configuration."""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

DIRECTORY = Path(__file__).resolve().parent
DEFAULT_OUTPUT = DIRECTORY.parents[2] / 'target' / 'todo-showcase'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument('--offline', action='store_true', help='Use cached Cargo dependencies only')
    args = parser.parse_args()
    output = args.output.resolve()
    compiler = shlex.split(os.environ.get('CC', 'cc'))
    if not compiler or not shutil.which(compiler[0]) or not shutil.which('cargo'):
        parser.error('Install a C compiler and the Rust toolchain')
    if not shutil.which('pkg-config'):
        parser.error('Install pkg-config and json-c development headers (see README.md)')
    dependency = subprocess.run(['pkg-config', '--atleast-version=0.15', 'json-c'])
    if dependency.returncode:
        parser.error('Install json-c 0.15+ development headers (see README.md)')
    flags = shlex.split(subprocess.check_output(['pkg-config', '--cflags', '--libs', 'json-c'], text=True))
    output.mkdir(parents=True, exist_ok=True)
    subprocess.run(compiler + ['-std=c11', '-O2', '-Wall', '-Wextra', '-Wpedantic', '-Werror',
                              str(DIRECTORY / 'c' / 'todo.c'), '-o', str(output / 'todo-c')] + flags, check=True)
    subprocess.run(['cargo', 'build', '--locked', '--release',
                    '--manifest-path', str(DIRECTORY / 'rust' / 'Cargo.toml'),
                    '--target-dir', str(output / 'rust')] + (['--offline'] if args.offline else []), check=True)
    plugins = []
    for language, executable, arguments in [
        ('python', Path(sys.executable).resolve(), [str(DIRECTORY / 'python' / 'todo.py')]),
        ('rust', output / 'rust' / 'release' / 'runyte-todo-example', []),
        ('c', output / 'todo-c', []),
    ]:
        plugins.append({'id': f'todo-{language}', 'enabled': True, 'api': 'runyte-experimental-2',
                        'executable': str(executable), 'args': arguments, 'capabilities': ['views']})
    config = output / 'config.yaml'
    config.write_text(json.dumps({'plugins': plugins}, indent=2) + '\n', encoding='utf-8')
    print(f'Built all variants. Demo configuration: {config}')


if __name__ == '__main__':
    main()
