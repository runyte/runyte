#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Builds the Runyte executable. Any extra arguments are forwarded to
# `cargo build`, so `./build.sh --release` produces a release build.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

exec cargo build --bins "$@"
