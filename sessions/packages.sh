#!/bin/sh
set -eu
[ "$#" -gt 0 ] || exit 0
for package do
    case "$package" in ''|-*|*-|*[!a-zA-Z0-9@._+:-]*) echo 'Invalid package name' >&2; exit 1;; esac
done
for package do
    shift
    if command -v pacman >/dev/null; then
        pacman -Q "$package" >/dev/null 2>&1 || set -- "$@" "$package"
    else
        [ "$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null || true)" = installed ] || set -- "$@" "$package"
    fi
done
[ "$#" -gt 0 ] || exit 0
if command -v pacman >/dev/null; then
    pacman -Syu --noconfirm --needed -- "$@"
else
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends -- "$@"
fi
