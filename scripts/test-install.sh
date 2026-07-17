#!/usr/bin/env bash
# Local end-to-end test for install.sh. Builds a fake release layout from an
# existing target/release/groky binary, serves it over a local HTTP server,
# and exercises both the happy path and the checksum-mismatch path.
#
# Usage: scripts/test-install.sh
# Requires: target/release/groky (cargo build --release -p xai-grok-pager-bin)
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
binary="$repo_root/target/release/groky"
[ -f "$binary" ] || {
    echo "error: $binary not found; build it first with:" >&2
    echo "  cargo build --release -p xai-grok-pager-bin" >&2
    exit 1
}

version="0.1.0"
tag="v$version"
os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Linux) os_triple="unknown-linux-gnu" ;;
    Darwin) os_triple="apple-darwin" ;;
    *)
        echo "error: unsupported test OS: $os" >&2
        exit 1
        ;;
esac
case "$arch" in
    x86_64 | amd64) arch="x86_64" ;;
    aarch64 | arm64) arch="aarch64" ;;
    *)
        echo "error: unsupported test arch: $arch" >&2
        exit 1
        ;;
esac
target="${arch}-${os_triple}"
tarball="groky-${version}-${target}.tar.gz"

workdir=$(mktemp -d)
server_pid=""
cleanup() {
    [ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null
    rm -rf "$workdir"
}
trap cleanup EXIT

# --- Build the fake release layout (mirrors the release workflow) ------------
staging="groky-${version}-${target}"
mkdir -p "$workdir/serve/$staging"
cp "$binary" "$repo_root/LICENSE" "$repo_root/THIRD-PARTY-NOTICES" \
    "$workdir/serve/$staging/"
(
    cd "$workdir/serve"
    tar czf "$tarball" "$staging"
    rm -rf "$staging"
    if command -v sha256sum >/dev/null; then
        sha256sum "$tarball" > "$tarball.sha256"
    else
        shasum -a 256 "$tarball" > "$tarball.sha256"
    fi
)

# --- Serve it over localhost --------------------------------------------------
port=8931
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$workdir/serve" \
    >/dev/null 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do
    curl -fsS "http://127.0.0.1:$port/" >/dev/null 2>&1 && break
    sleep 0.1
done

base="http://127.0.0.1:$port"

# --- Happy path ----------------------------------------------------------------
install_dir=$(mktemp -d)
GROKY_DOWNLOAD_BASE="$base" GROKY_VERSION="$tag" \
    GROKY_INSTALL_DIR="$install_dir" bash "$repo_root/install.sh"

out=$("$install_dir/groky" --version)
case "$out" in
    *groky*) echo "PASS: installed binary runs ($out)" ;;
    *)
        echo "FAIL: unexpected --version output: $out" >&2
        exit 1
        ;;
esac

# --- Checksum mismatch must fail ------------------------------------------------
echo "corrupt" >> "$workdir/serve/$tarball"
if GROKY_DOWNLOAD_BASE="$base" GROKY_VERSION="$tag" \
    GROKY_INSTALL_DIR="$(mktemp -d)" bash "$repo_root/install.sh" \
    >/dev/null 2>&1; then
    echo "FAIL: installer accepted a corrupted tarball" >&2
    exit 1
fi
echo "PASS: checksum mismatch rejected"

echo "All installer tests passed."
