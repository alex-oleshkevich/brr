#!/bin/sh
# Install the brr binary from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/alex-oleshkevich/brr/main/install.sh | sh
#
# BRR_VERSION=0.1.0     install that version instead of the latest
# BRR_INSTALL_DIR=DIR   install into DIR instead of ~/.local/bin
set -eu

repo="alex-oleshkevich/brr"
dir="${BRR_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "error: $*" >&2
    exit 1
}

[ "$(uname -s)" = "Linux" ] || fail "brr runs on Linux only"

case "$(uname -m)" in
    x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
    aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
    *) fail "no brr build for $(uname -m); build from source with: just install" ;;
esac

command -v curl >/dev/null || fail "curl is required"
command -v tar >/dev/null || fail "tar is required"
command -v sha256sum >/dev/null || fail "sha256sum is required"

version="${BRR_VERSION:-}"
if [ -z "$version" ]; then
    latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
    version="${latest##*/v}"
    [ -n "$version" ] && [ "$version" != "$latest" ] || fail "cannot find the latest release of $repo"
fi
version="${version#v}"

name="brr-$version-$target"
url="https://github.com/$repo/releases/download/v$version/$name.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "downloading brr $version for $target"
curl -fsSL -o "$tmp/$name.tar.gz" "$url" || fail "cannot download $url"
curl -fsSL -o "$tmp/$name.tar.gz.sha256" "$url.sha256" || fail "cannot download $url.sha256"
(cd "$tmp" && sha256sum -c --quiet "$name.tar.gz.sha256") || fail "checksum mismatch for $name.tar.gz"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$dir"
# install replaces the file instead of writing into it, so a running daemon keeps its old binary.
install -m 755 "$tmp/$name/brr" "$dir/brr"
echo "installed $dir/brr ($("$dir/brr" version))"

case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "note: $dir is not on your PATH; add it, for example: export PATH=\"$dir:\$PATH\"" ;;
esac

if "$dir/brr" daemon status >/dev/null 2>&1; then
    echo "a daemon is running the old binary; restart it: brr daemon stop && brr daemon start"
else
    echo "next: brr doctor"
fi
