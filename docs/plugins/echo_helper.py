# SPDX-License-Identifier: MPL-2.0
"""Deterministic managed-process backend for helper.py; stdout is ordinary data."""
import sys


def run():
    output = sys.stdout.buffer
    output.write(b'Helper ready. Send a line, flood, or EOF.\n')
    output.flush()
    while True:
        line = sys.stdin.buffer.readline(65537)
        if not line:
            return
        if len(line) > 65536:
            output.write(b'Input line exceeded the example limit.\n')
            output.flush()
            return
        if line.strip() == b'flood':
            for index in range(32768):
                output.write(f'Flood record {index:05}: bounded retained output.\n'.encode())
        else:
            output.write(b'Echo: ' + line)
        output.flush()


if __name__ == '__main__':
    run()
