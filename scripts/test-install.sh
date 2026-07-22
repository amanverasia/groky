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
install_dir=""
install_dir2=""
install_dir3=""
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$workdir" "$install_dir" "$install_dir2" "$install_dir3"
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
    cp "$tarball" "$tarball.pristine"
)

# --- Serve it over localhost --------------------------------------------------
# Random high port; retry once in case of a collision with another process.
start_server() {
    port=$(( (RANDOM % 20000) + 20000 ))
    python3 -m http.server "$port" --bind 127.0.0.1 \
        --directory "$workdir/serve" >/dev/null 2>&1 &
    server_pid=$!
    for _ in $(seq 1 50); do
        if curl -fsS "http://127.0.0.1:$port/" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.1
    done
    kill "$server_pid" 2>/dev/null || true
    server_pid=""
    return 1
}
start_server || start_server || {
    echo "error: could not start a local http server" >&2
    exit 1
}

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
install_dir2=$(mktemp -d)
if GROKY_DOWNLOAD_BASE="$base" GROKY_VERSION="$tag" \
    GROKY_INSTALL_DIR="$install_dir2" bash "$repo_root/install.sh" \
    >/dev/null 2>&1; then
    echo "FAIL: installer accepted a corrupted tarball" >&2
    exit 1
fi
echo "PASS: checksum mismatch rejected"

# --- Checksum sidecar must name exactly this tarball ----------------------------
cp "$workdir/serve/$tarball.pristine" "$workdir/serve/$tarball"
reject_sidecar() {
    install_dir3=$(mktemp -d)
    if GROKY_DOWNLOAD_BASE="$base" GROKY_VERSION="$tag" \
        GROKY_INSTALL_DIR="$install_dir3" bash "$repo_root/install.sh" \
        >/dev/null 2>&1; then
        echo "FAIL: installer accepted $1" >&2
        exit 1
    fi
    rm -rf "$install_dir3"
    install_dir3=""
}

# A sidecar can otherwise make sha256sum check a trusted system file instead.
printf '%s  /dev/null\n' \
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" \
    > "$workdir/serve/$tarball.sha256"
reject_sidecar "a checksum sidecar naming /dev/null"

checksum=$(if command -v sha256sum >/dev/null; then
    sha256sum "$workdir/serve/$tarball" | cut -d ' ' -f1
else
    shasum -a 256 "$workdir/serve/$tarball" | cut -d ' ' -f1
fi)
printf '%s  -\n' "$checksum" > "$workdir/serve/$tarball.sha256"
reject_sidecar "a checksum sidecar naming -"
printf 'not-a-checksum\n' > "$workdir/serve/$tarball.sha256"
reject_sidecar "a malformed checksum sidecar"
printf '%s  %s\n%s  %s\n' "$checksum" "$tarball" "$checksum" "$tarball" \
    > "$workdir/serve/$tarball.sha256"
reject_sidecar "a multiple-line checksum sidecar"
echo "PASS: invalid checksum sidecars rejected"

echo "All installer tests passed."
