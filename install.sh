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

# The Developer ID team that signs release apps. The team, unlike a
# certificate, survives renewal; context/reference/releasing.md records it.
TEAM_ID=DP96UQZ969

cleanup() {
    # An interruption while the previous app is set aside puts it back if the
    # new one is not yet in place, and otherwise discards it. A removal that
    # fails must not stop the rest of the cleanup.
    if [ -n "$previous" ]; then
        if [ -e "$app" ] || [ -L "$app" ]; then
            rm -rf "$previous" || :
        elif mv "$previous/Runyte.app" "$app" 2>/dev/null; then
            rm -rf "$previous" || :
        fi
    fi
    if [ -n "$stage" ]; then rm -rf "$stage" || :; fi
    if [ -n "$link_stage" ]; then rm -f "$link_stage" || :; fi
    if [ -n "$work" ]; then rm -rf "$work" || :; fi
}

# Whether the dynamic loader can find every library the Linux desktop
# executable links directly. Without ldconfig there is nothing to consult, and
# the executable reports anything missing itself when it starts.
check_desktop_libraries() {
    cache=
    for ldconfig in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
        command -v "$ldconfig" >/dev/null 2>&1 || continue
        cache=$("$ldconfig" -p 2>/dev/null) || cache=
        break
    done
    [ -n "$cache" ] || return 0
    missing=
    for library in libxcb.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0; do
        case "$cache" in
            *"$library ("*) continue ;;
        esac
        # Libraries outside the loader cache, as with Nix, come from
        # LD_LIBRARY_PATH.
        found=
        set -f
        old_ifs=$IFS
        IFS=:
        for directory in ${LD_LIBRARY_PATH:-}; do
            if [ -n "$directory" ] && [ -e "$directory/$library" ]; then found=1; fi
        done
        IFS=$old_ifs
        set +f
        [ -n "$found" ] || missing="$missing $library"
    done
    [ -z "$missing" ] || fail "The desktop edition needs$missing; install the libraries listed at https://github.com/runyte/runyte/blob/main/INSTALLATION.md#linux-desktop-dependencies or use --terminal"
}

# Refuses, before anything is downloaded, an application directory inside a
# bundle or an existing Runyte.app that this installer must not replace.
check_app_destination() {
    while [ "${app_dir%/}" != "$app_dir" ] && [ "$app_dir" != / ]; do app_dir=${app_dir%/}; done
    case "$app_dir" in
        *.app|*.app/*) fail "The application directory must not be inside an application bundle: $app_dir" ;;
    esac
    app=${app_dir%/}/Runyte.app
    if [ -e "$app" ] || [ -L "$app" ]; then
        { [ -d "$app" ] && [ ! -L "$app" ]; } || fail "Not an application bundle: $app"
        awk '/<string>com\.runyte\.Runyte<\/string>/ { found=1 } END { exit !found }' \
            "$app/Contents/Info.plist" 2>/dev/null \
            || fail "Refusing to replace an application that is not Runyte: $app"
    fi
    # /Applications, for example, is writable only by administrators.
    if [ -d "$app_dir" ] && [ ! -w "$app_dir" ]; then
        fail "Cannot write to $app_dir; choose another directory with --app-dir"
    fi
}

# Replaces $app with the archive's app. The previous app waits outside the
# staging directory, so no failure or interruption deletes it, and is put back
# if the new one cannot take its place.
install_app() {
    tar -xJf "$work/$archive" -C "$work" "$package/Runyte.app" \
        || fail 'Could not extract Runyte.app (tar with xz support is required)'
    new=$work/$package/Runyte.app
    { [ -f "$new/Contents/MacOS/runyte" ] && [ ! -L "$new/Contents/MacOS/runyte" ]; } \
        || fail 'The archive contains no Runyte.app executable'
    # A download carries no quarantine, so Gatekeeper never assesses it on
    # launch. Require the release team's Developer ID Application certificate,
    # which development certificates of the same team do not satisfy, then
    # Gatekeeper's own assessment, which needs the app's notarization.
    codesign --verify --deep --strict \
        "-R=anchor apple generic and identifier \"com.runyte.Runyte\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$TEAM_ID\"" \
        "$new" \
        || fail 'The downloaded Runyte.app is not signed by the Runyte release team; existing installation was not changed'
    spctl --assess --type execute "$new" \
        || fail 'Gatekeeper rejected the downloaded Runyte.app; existing installation was not changed'
    mkdir -p "$app_dir" || fail "Could not create $app_dir"
    stage=$(mktemp -d "$app_dir/.runyte-install.XXXXXXXX") || fail "Cannot write to $app_dir"
    mv "$new" "$stage/Runyte.app" || fail "Could not stage Runyte.app in $app_dir"
    if [ -d "$app" ]; then
        previous=$(mktemp -d "$app_dir/.runyte-previous.XXXXXXXX") || fail "Cannot write to $app_dir"
        if ! mv "$app" "$previous/Runyte.app"; then
            rm -rf "$previous" || :
            previous=
            fail "Could not replace $app"
        fi
    fi
    if ! mv "$stage/Runyte.app" "$app"; then
        if [ -n "$previous" ]; then
            if ! mv "$previous/Runyte.app" "$app"; then
                kept=$previous
                previous=
                fail "Could not replace $app; the previous app is in $kept"
            fi
            rm -rf "$previous" || :
            previous=
        fi
        fail "Could not replace $app"
    fi
    # The new app is in place, so failing to discard the old one, for example
    # a file the user cannot delete, must not stop the link to the new one.
    if [ -n "$previous" ]; then
        rm -rf "$previous" \
            || printf 'Could not remove the previous app in %s; remove it yourself.\n' "$previous"
        previous=
    fi
    rm -rf "$stage" || :
    stage=
    printf 'Installed Runyte.app %s to %s\n' "$version" "$app"
    # A relative name would resolve against $install_dir, so link absolutely.
    link_stage=$install_dir/.runyte-install.$$
    rm -f "$link_stage"
    ln -s "$app/Contents/MacOS/runyte" "$link_stage" || fail "Cannot write to $install_dir"
    mv -f "$link_stage" "$destination" || fail "Could not replace $destination"
    link_stage=
    printf 'Linked %s to %s\n' "$destination" "$app/Contents/MacOS/runyte"
    if [ -n "$linked_app" ] && [ -d "$linked_app" ] && ! [ "$linked_app" -ef "$app" ]; then
        printf 'The previously linked app %s was left in place; remove it if you no longer need it.\n' "$linked_app"
    fi
}

main() {
    version=
    edition=
    app_dir=
    app=
    previous=
    install_dir=${HOME:-}/.local/bin
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version|--install-dir|--app-dir)
                [ "$#" -ge 2 ] || fail "Missing value for $1"
                [ -n "$2" ] || fail "Empty value for $1"
                case "$1" in
                    --version) version=$2 ;;
                    --install-dir) install_dir=$2 ;;
                    --app-dir) app_dir=$2 ;;
                esac
                shift 2
                ;;
            --desktop|--terminal)
                [ -z "$edition" ] || [ "$edition" = "${1#--}" ] || fail 'Choose either --desktop or --terminal'
                edition=${1#--}
                shift
                ;;
            --help|-h)
                printf '%s\n' \
                    'Install or update Runyte from GitHub Releases.' \
                    'Usage: sh install.sh [--desktop | --terminal] [--version X.Y.Z]' \
                    '                     [--install-dir /absolute/path] [--app-dir /absolute/path]' \
                    'Default: the latest release of the edition already installed in the install' \
                    'directory, otherwise the terminal edition, as $HOME/.local/bin/runyte.' \
                    '--desktop installs the desktop edition. On macOS it installs Runyte.app in' \
                    '$HOME/Applications, or --app-dir, and links runyte to it.' \
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
    case "$app_dir" in
        ''|/*) ;;
        *) fail 'The application directory must be an absolute path' ;;
    esac
    [ -n "${HOME:-}" ] || [ "$install_dir" != /.local/bin ] || fail 'HOME is unset; use --install-dir'

    for tool in curl uname awk tar mktemp mkdir chmod mv rm; do
        command -v "$tool" >/dev/null 2>&1 || fail "Required tool is missing: $tool"
    done
    # The edition already installed is the one an update keeps, unless an
    # option chooses. A desktop app on macOS is linked rather than copied, so
    # that link is the one symlink this installer will replace.
    destination=$install_dir/runyte
    installed=
    linked_app=
    if [ -L "$destination" ]; then
        linked_app=$(readlink "$destination") || fail "Could not read $destination"
        case "$linked_app" in
            /*/Runyte.app/Contents/MacOS/runyte) installed=desktop ;;
            *) fail "Refusing to replace a symlink: $destination" ;;
        esac
        linked_app=${linked_app%/Contents/MacOS/runyte}
    elif [ -e "$destination" ] && [ ! -f "$destination" ]; then
        fail "Not a regular file: $destination"
    elif [ -x "$destination" ]; then
        installed_version=$("$destination" --version 2>/dev/null) || installed_version=
        case "$installed_version" in
            *' (desktop edition)') installed=desktop ;;
            ?*) installed=terminal ;;
            *)
                [ -n "$edition" ] \
                    || fail "Could not tell which edition $destination is because its --version failed; rerun with --desktop or --terminal"
                ;;
        esac
    fi
    [ -n "$edition" ] || edition=${installed:-terminal}

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
            # Terminal archives are built on Ubuntu 22.04 and desktop archives
            # on Ubuntu 24.04; each keeps that release's glibc floor.
            floor=35
            if [ "$edition" = desktop ]; then floor=39; fi
            libc=$(getconf GNU_LIBC_VERSION 2>/dev/null) || fail "The $edition edition requires glibc 2.$floor or newer; musl is unsupported"
            printf '%s\n' "$libc" | awk -v floor="$floor" '
                $1 == "glibc" { split($2, v, "."); if (v[1] > 2 || (v[1] == 2 && v[2] >= floor)) ok=1 }
                END { exit !ok }
            ' || fail "The $edition edition requires glibc 2.$floor or newer (found $libc)"
            [ -z "$app_dir" ] || fail '--app-dir is only used by the macOS desktop edition'
            if [ "$edition" = desktop ]; then check_desktop_libraries; fi
            ;;
        Darwin)
            target=$arch-apple-darwin
            if [ "$edition" = desktop ]; then
                # One universal app serves Apple silicon and Intel Macs.
                target=universal-apple-darwin
                if command -v sw_vers >/dev/null 2>&1; then
                    macos=$(sw_vers -productVersion) || fail 'Could not read the macOS version'
                    [ "${macos%%.*}" -ge 11 ] 2>/dev/null || fail "The desktop edition requires macOS 11 or newer (found $macos)"
                fi
                for tool in codesign spctl; do
                    command -v "$tool" >/dev/null 2>&1 || fail "Required tool is missing: $tool"
                done
                if [ -z "$app_dir" ]; then
                    if [ -n "$linked_app" ]; then
                        app_dir=${linked_app%/Runyte.app}
                    else
                        [ -n "${HOME:-}" ] || fail 'HOME is unset; use --app-dir'
                        app_dir=$HOME/Applications
                    fi
                fi
                check_app_destination
            else
                [ -z "$app_dir" ] || fail '--app-dir is only used by the macOS desktop edition'
            fi
            ;;
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
    link_stage=
    trap cleanup 0
    trap 'exit 1' HUP INT TERM
    work=$(mktemp -d "${TMPDIR:-/tmp}/runyte-install.XXXXXXXX") || fail 'Could not create a temporary directory'
    base=$releases/download/v$version
    printf 'Downloading Runyte %s (%s edition) for %s...\n' "$version" "$edition" "$target"
    download --output "$work/SHA256SUMS" "$base/SHA256SUMS" || fail 'Could not download SHA256SUMS'

    if [ "$edition" = desktop ]; then
        prefix=runyte-desktop
        awk -v name="$prefix-v$version-$target.tar.xz" '$2 == name { found=1 } END { exit !found }' \
            "$work/SHA256SUMS" \
            || fail "Runyte $version has no desktop edition for $target (see https://github.com/runyte/runyte#editions)"
    else
        # The manifest, not the version number, defines historical archive layouts.
        prefix=$(awk -v new="runyte-terminal-v$version-$target.tar.xz" \
            -v old="runyte-v$version-$target.tar.xz" '
            $2 == new { modern=1 }
            $2 == old { historical=1 }
            END {
                if (modern) print "runyte-terminal"
                else if (historical) print "runyte"
                else exit 1
            }
        ' "$work/SHA256SUMS") || fail "SHA256SUMS lists no terminal archive for $target"
    fi
    archive=$prefix-v$version-$target.tar.xz
    package=$prefix-$version-$target
    download --output "$work/$archive" "$base/$archive" || fail "Could not download $archive"

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
    if [ "$edition" = desktop ] && [ "$os" = Darwin ]; then
        install_app
    else
        # Stage on the destination filesystem, then rename, so updates also
        # work while the old executable is running and failures leave it intact.
        stage=$(mktemp "$install_dir/.runyte-install.XXXXXXXX") || fail "Cannot write to $install_dir"
        # Read only the executable to stdout; archive paths are never extracted.
        tar -xOJf "$work/$archive" "$package/runyte" > "$stage" \
            || fail 'Could not extract runyte (tar with xz support is required)'
        [ -s "$stage" ] || fail 'The archive contains no executable data'
        chmod 755 "$stage" || fail 'Could not set executable permissions'
        mv -f "$stage" "$destination" || fail "Could not replace $destination"
        stage=
        printf 'Installed Runyte %s to %s\n' "$version" "$destination"
    fi
    if [ "$edition" = desktop ] && [ "$os" = Linux ]; then
        # The launcher records the installed path, which updates keep. The
        # registration script and icons come from the verified archive.
        if ! command -v python3 >/dev/null 2>&1; then
            printf '%s\n' 'Python 3 is required to register a desktop launcher; runyte --window works without one.'
        elif registration=$({
            tar -xJf "$work/$archive" -C "$work" "$package/contrib/packaging" \
                && python3 "$work/$package/contrib/packaging/package.py" linux --binary "$destination" >/dev/null
        } 2>&1); then
            printf '%s\n' 'Registered the Runyte desktop launcher and icons'
        else
            printf 'Could not register a desktop launcher (%s); runyte --window works without one.\n' \
                "$(printf '%s\n' "$registration" | awk 'NF { line=$0 } END { print line }')"
        fi
    fi
    if [ -n "$installed" ] && [ "$installed" != "$edition" ]; then
        printf 'Replaced the %s edition with the %s edition.\n' "$installed" "$edition"
        if [ -n "$linked_app" ] && [ "$edition" = terminal ]; then
            printf 'The desktop app %s was left in place; remove it if you no longer need it.\n' "$linked_app"
        fi
    fi
    # runed is the same executable started under another name, which runs it
    # as runyte --editor. A relative link keeps following updates in place.
    # An absolute link into a Runyte.app, as the disk-image instructions make,
    # becomes that link so runed cannot outlive an edition switch. Anything
    # else already called runed is someone else's and is left alone.
    link=$install_dir/runed
    runed_target=
    if [ -L "$link" ]; then runed_target=$(readlink "$link") || runed_target=; fi
    if [ "$runed_target" = runyte ]; then
        :
    elif case "$runed_target" in /*/Runyte.app/Contents/MacOS/runyte) true ;; *) false ;; esac; then
        link_stage=$install_dir/.runyte-install.$$
        rm -f "$link_stage"
        if ln -s runyte "$link_stage" && mv -f "$link_stage" "$link"; then
            link_stage=
            printf 'Linked %s to runyte for editor mode\n' "$link"
        else
            printf 'Could not replace %s; runyte --editor works the same.\n' "$link"
        fi
    elif [ -e "$link" ] || [ -L "$link" ]; then
        printf 'Left %s unchanged; runed would be a link to runyte.\n' "$link"
    elif ln -s runyte "$link"; then
        printf 'Linked %s to runyte for editor mode\n' "$link"
    else
        printf 'Could not create %s; runyte --editor works the same.\n' "$link"
    fi
    case ":${PATH:-}:" in
        *":$install_dir:"*) ;;
        *) printf 'Add %s to your PATH to run it as runyte.\n' "$install_dir" ;;
    esac
    printf '%s\n' 'Run this installer again with the same install directory to update.'
}

# Keeping installation inside main prevents a truncated piped download from
# executing an incomplete installation procedure.
main "$@"
