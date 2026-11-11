#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Prepare a temporary, CPU-only GPUI scene replay experiment.

The vendored GPUI is not a workspace member. Its normal test graph additionally
needs upstream test-support dependencies. This copies the real crate, removes
only that unused test dependency graph, and exposes the scene equality test and
benchmark through a temporary normal binary. No tracked manifest is changed.
Build and run the printed commands serially, after other builds/tests finish.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import shutil
import tempfile
import tomllib

REPO = Path(__file__).resolve().parent.parent
FUNCTIONS = ('layered_replay_preserves_order_masks_path_ids_and_replayed_operations',
             'layered_replay_benchmark')


def prepare(destination):
    source = REPO / 'vendor/gpui'
    target = destination / 'gpui'
    shutil.copytree(source, target)
    manifest = target / 'Cargo.toml'
    sections = re.split(r'(?=^\[)', manifest.read_text(), flags=re.M)
    contents = ''.join(section for section in sections if not re.match(
        r'\[(?:target\..*\.)?dev-dependencies(?:\.|\])', section))
    # Cargo resolves a root package's optional feature graph even though this
    # binary does not enable test-support. Keep the cfg name but no dependencies.
    contents = re.sub(r'^test-support = \[.*?^\]\n', 'test-support = []\n',
                      contents, flags=re.M | re.S)
    profile = tomllib.loads((REPO / 'Cargo.toml').read_text())['profile']['release']
    contents += '\n[workspace]\n\n[profile.release]\n'
    for key, value in profile.items():
        contents += f'{key} = {json.dumps(value)}\n'
    contents += '\n[patch.crates-io]\n'
    for name in ('block', 'proc-macro-error2', 'stylo_derive'):
        contents += f'{name} = {{ path = {json.dumps(str(REPO / "vendor" / name))} }}\n'
    contents += '\n[[bin]]\nname = "runyte-scene-replay"\npath = "runyte_scene_replay.rs"\n'
    manifest.write_text(contents)
    shutil.copyfile(REPO / 'Cargo.lock', target / 'Cargo.lock')
    experiment = source / 'src/scene/replay_tests.rs'
    contents = experiment.read_text().replace('#[test]\n', '')
    contents = re.sub(r'^#\[ignore = .*\]\n', '', contents, flags=re.M)
    for name in FUNCTIONS:
        contents = contents.replace('fn ' + name + '(', 'pub(crate) fn ' + name + '(')
    (target / 'src/scene/replay_tests.rs').write_text(contents)
    scene = target / 'src/scene.rs'
    contents = scene.read_text()
    original = '#[cfg(test)]\n#[path = "scene/replay_tests.rs"]\nmod replay_tests;'
    if original not in contents:
        raise ValueError('scene experiment module declaration changed; review harness')
    scene.write_text(contents.replace(original,
        '#[path = "scene/replay_tests.rs"]\npub(crate) mod replay_tests;'))
    with (target / 'src/gpui.rs').open('a') as file:
        file.write('\n/// Run the temporary CPU-only replay equality check and timing experiment.\n'
                   'pub fn runyte_scene_replay_experiment() {\n')
        for name in FUNCTIONS:
            file.write(f'    scene::replay_tests::{name}();\n')
        file.write('}\n')
    (target / 'runyte_scene_replay.rs').write_text(
        'fn main() { gpui::runyte_scene_replay_experiment(); }\n')
    paths = ['Cargo.toml', 'Cargo.lock', 'vendor/gpui/Cargo.toml',
             'vendor/gpui/src/scene.rs', 'vendor/gpui/src/scene/replay_tests.rs',
             'vendor/gpui/src/bounds_tree.rs', 'vendor/gpui/src/geometry.rs']
    hashes = {path: hashlib.sha256((REPO / path).read_bytes()).hexdigest() for path in paths}
    (destination / 'source-hashes.json').write_text(json.dumps(hashes, indent=2) + '\n')
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, help='new or existing empty experiment directory')
    args = parser.parse_args()
    destination = args.directory or Path(tempfile.mkdtemp(prefix='runyte-gpui-replay-'))
    destination.mkdir(parents=True, exist_ok=True)
    target = prepare(destination.resolve())
    manifest = shlex.quote(str(target / 'Cargo.toml'))
    target_dir = shlex.quote(str(REPO / 'target'))
    print(f'cargo metadata --offline --format-version 1 --manifest-path {manifest} > /dev/null')
    print(f'CARGO_TARGET_DIR={target_dir} cargo build --offline --locked --release '
          f'--manifest-path {manifest} --bin runyte-scene-replay')
    print(shlex.quote(str(REPO / 'target/release/runyte-scene-replay')))
    print(f'Source hashes: {destination / "source-hashes.json"}')


if __name__ == '__main__':
    main()
