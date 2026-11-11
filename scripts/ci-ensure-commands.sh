#!/bin/bash
# Installs Debian packages for commands that are not already on PATH.
# usage: scripts/ci-ensure-commands.sh command:package [command:package...]
# A persistent runner keeps the packages, so a later job does not call apt.
# A container image that lacks the tools still installs them. sudo never prompts.
set -euo pipefail

if [ "$#" -lt 1 ]; then
    echo "usage: scripts/ci-ensure-commands.sh command:package ..." >&2
    exit 2
fi

# Space-separated package names. Bash 3.2 has no associative arrays, and macOS CI runs this test.
pkgs=""
for spec in "$@"; do
    cmd="${spec%%:*}"
    pkg="${spec#*:}"
    if [ -z "$cmd" ] || [ -z "$pkg" ] || [ "$cmd" = "$spec" ]; then
        echo "bad spec (want command:package): $spec" >&2
        exit 2
    fi
    if [ -x "$cmd" ] || command -v -- "$cmd" >/dev/null 2>&1; then
        continue
    fi
    case " $pkgs " in
        *" $pkg "*) ;;
        *) pkgs="$pkgs $pkg" ;;
    esac
done
pkgs="${pkgs# }"

if [ -z "$pkgs" ]; then
    exit 0
fi

echo "installing missing tools: $pkgs" >&2

if [ "$(id -u)" -eq 0 ]; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    # Word-splitting is the package list.
    # shellcheck disable=SC2086
    apt-get install -y --no-install-recommends $pkgs
elif command -v sudo >/dev/null 2>&1 && sudo -n true; then
    sudo -n env DEBIAN_FRONTEND=noninteractive apt-get update
    # shellcheck disable=SC2086
    sudo -n env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends $pkgs
else
    echo "cannot install $pkgs without root or passwordless sudo" >&2
    exit 1
fi
