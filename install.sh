#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
# Install or update Runyte from the official GitHub Release archives.

set -eu

fail() {
    printf 'runyte installer: %s\n' "$*" >&2
    exit 1
}

download() {
    # Ignore ~/.curlrc and refuse redirects to an unencrypted transport.
    curl -q --fail --silent --show-error --location \
        --proto '=https' --proto-redir '=https' --connect-timeout 15 \
        --max-time 300 --retry 2 "$@"
}

cleanup() {
    if [ -n "$stage" ]; then rm -f "$stage"; fi
    if [ -n "$work" ]; then rm -rf "$work"; fi
}

main() {
    version=
    install_dir=${HOME:-}/.local/bin
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version|--install-dir)
                [ "$#" -ge 2 ] || fail "Missing value for $1"
                [ -n "$2" ] || fail "Empty value for $1"
                case "$1" in
                    --version) version=$2 ;;
                    --install-dir) install_dir=$2 ;;
                esac
                shift 2
                ;;
            --help|-h)
                printf '%s\n' \
                    'Install or update Runyte from GitHub Releases.' \
                    'Usage: sh install.sh [--version X.Y.Z] [--install-dir /absolute/path]' \
                    'Default: latest release, installed to $HOME/.local/bin/runyte.' \
                    'Run the same command again to update. No sudo is needed.'
                return
                ;;
            *) fail "Unknown argument: $1 (see --help)" ;;
        esac
    done
    case "$install_dir" in
        /*) ;;
        *) fail 'The install directory must be an absolute path' ;;
    esac
    [ -n "${HOME:-}" ] || [ "$install_dir" != /.local/bin ] || fail 'HOME is unset; use --install-dir'

    for tool in curl uname awk tar mktemp mkdir chmod mv rm; do
        command -v "$tool" >/dev/null 2>&1 || fail "Required tool is missing: $tool"
    done
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) fail "Unsupported architecture: $arch (requires x86-64 or ARM64)" ;;
    esac
    case "$os" in
        Linux)
            target=$arch-unknown-linux-gnu
            # The release workflow builds against the Ubuntu 22.04 glibc floor.
            libc=$(getconf GNU_LIBC_VERSION 2>/dev/null) || fail 'Linux requires glibc 2.35 or newer; musl is unsupported'
            printf '%s\n' "$libc" | awk '
                $1 == "glibc" { split($2, v, "."); if (v[1] > 2 || (v[1] == 2 && v[2] >= 35)) ok=1 }
                END { exit !ok }
            ' || fail "Linux requires glibc 2.35 or newer (found $libc)"
            ;;
        Darwin) target=$arch-apple-darwin ;;
        *) fail "Unsupported system: $os (requires Linux or macOS)" ;;
    esac
    if command -v sha256sum >/dev/null 2>&1; then
        checksum=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        checksum=shasum
    else
        fail 'Required tool is missing: sha256sum or shasum'
    fi

    releases=https://github.com/runyte/runyte/releases
    if [ -z "$version" ]; then
        latest=$(download --output /dev/null --write-out '%{url_effective}' "$releases/latest") \
            || fail 'Could not determine the latest release; retry or use --version X.Y.Z'
        case "$latest" in
            "$releases/tag/"*) version=${latest#"$releases/tag/"} ;;
            *) fail "Unexpected latest release URL: $latest" ;;
        esac
    fi
    version=${version#v}
    printf '%s\n' "$version" | awk '
        /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/ { ok=1 }
        END { exit !(ok && NR == 1) }
    ' || fail 'Version must be X.Y.Z or vX.Y.Z'

    work=
    stage=
    trap cleanup 0
    trap 'exit 1' HUP INT TERM
    work=$(mktemp -d "${TMPDIR:-/tmp}/runyte-install.XXXXXXXX") || fail 'Could not create a temporary directory'
    archive=runyte-v$version-$target.tar.xz
    base=$releases/download/v$version
    printf 'Downloading Runyte %s for %s...\n' "$version" "$target"
    download --output "$work/$archive" "$base/$archive" || fail "Could not download $archive"
    download --output "$work/SHA256SUMS" "$base/SHA256SUMS" || fail 'Could not download SHA256SUMS'

    # Match exactly one entry, never ask a checksum tool to read manifest paths.
    expected=$(awk -v name="$archive" '
        $2 == name { count++; digest=$1; if (NF != 2) bad=1 }
        END {
            if (count != 1 || bad || length(digest) != 64 || digest ~ /[^0-9a-fA-F]/) exit 1
            print tolower(digest)
        }
    ' "$work/SHA256SUMS") || fail "Missing, duplicate or malformed checksum for $archive"
    if [ "$checksum" = sha256sum ]; then
        actual=$(sha256sum < "$work/$archive") || fail 'Could not compute SHA-256'
    else
        actual=$(shasum -a 256 < "$work/$archive") || fail 'Could not compute SHA-256'
    fi
    actual=${actual%% *}
    [ "$actual" = "$expected" ] || fail "SHA-256 mismatch for $archive; existing installation was not changed"

    mkdir -p "$install_dir" || fail "Could not create $install_dir"
    destination=$install_dir/runyte
    [ ! -L "$destination" ] || fail "Refusing to replace a symlink: $destination"
    [ ! -e "$destination" ] || [ -f "$destination" ] || fail "Not a regular file: $destination"
    # Stage on the destination filesystem, then rename, so updates also work
    # while the old executable is running and failures leave it intact.
    stage=$(mktemp "$install_dir/.runyte-install.XXXXXXXX") || fail "Cannot write to $install_dir"
    # Read only the executable to stdout; archive paths are never extracted.
    tar -xOJf "$work/$archive" "runyte-$version-$target/runyte" > "$stage" \
        || fail 'Could not extract runyte (tar with xz support is required)'
    [ -s "$stage" ] || fail 'The archive contains no executable data'
    chmod 755 "$stage" || fail 'Could not set executable permissions'
    mv -f "$stage" "$destination" || fail "Could not replace $destination"
    stage=
    printf 'Installed Runyte %s to %s\n' "$version" "$destination"
    case ":${PATH:-}:" in
        *":$install_dir:"*) ;;
        *) printf 'Add %s to your PATH to run it as runyte.\n' "$install_dir" ;;
    esac
    printf '%s\n' 'Run this installer again with the same install directory to update.'
}

# Keeping installation inside main prevents a truncated piped download from
# executing an incomplete installation procedure.
main "$@"
