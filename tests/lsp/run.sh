#!/bin/sh
set -eu

repository=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
image=runyte-lsp-matrix

docker build --tag "$image" --file "$repository/tests/lsp/Dockerfile" "$repository"
docker run --rm "$image"
