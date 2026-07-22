#!/usr/bin/env bash
# Groky release installer.  GROK_* names are retained for compatibility.
set -euo pipefail

TARGET="${1:-${GROK_VERSION:-}}"
if [[ -n "$TARGET" && ! "$TARGET" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9._]+)?$ ]]; then
    echo "Invalid version format: $TARGET" >&2
    exit 1
fi

case "$(uname -s)" in
    Linux) target_os="unknown-linux-gnu" ;;
    MINGW*|MSYS*|CYGWIN*) echo "Windows is not supported: Windows release assets are not available." >&2; exit 1 ;;
    *) echo "Unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    x86_64|amd64|AMD64) arch="x86_64" ;;
    aarch64|arm64|ARM64) arch="aarch64" ;;
    *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }
command -v tar >/dev/null || { echo "tar is required" >&2; exit 1; }
if command -v sha256sum >/dev/null; then
    checksum_cmd=(sha256sum)
elif command -v shasum >/dev/null; then
    checksum_cmd=(shasum -a 256)
else
    echo "sha256sum or shasum is required" >&2
    exit 1
fi

if [[ -z "$TARGET" ]]; then
    TARGET=$(curl -fsSL https://api.github.com/repos/amanverasia/groky/releases/latest | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/.*"\(v[^"]*\)"$/\1/')
    [[ -n "$TARGET" ]] || { echo "Unable to determine the latest release; set GROK_VERSION." >&2; exit 1; }
fi
tag="${TARGET#v}"
release_tag="v${tag}"
target="${arch}-${target_os}"
tarball="groky-${tag}-${target}.tar.gz"
base="https://github.com/amanverasia/groky/releases/download/${release_tag}"
# Keep the installer layout aligned with the runtime managed-install path.
# GROK_HOME and GROK_BIN_DIR remain accepted for existing installations.
groky_home="${GROKY_HOME:-${GROK_HOME:-$HOME/.groky}}"
download_dir="$groky_home/downloads"
bin_dir="${GROK_BIN_DIR:-$groky_home/bin}"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL -o "$tmpdir/$tarball" "$base/$tarball"
curl -fsSL -o "$tmpdir/$tarball.sha256" "$base/$tarball.sha256"

# The checksum sidecar is untrusted input. Do not let a filename in it choose
# which file is verified: accept exactly one digest for this expected tarball,
# then compare that digest with the hash calculated from the downloaded bytes.
mapfile -t checksum_lines < "$tmpdir/$tarball.sha256"
if [[ ${#checksum_lines[@]} -ne 1 ]] ||
    [[ ! "${checksum_lines[0]}" =~ ^([[:xdigit:]]{64})[[:space:]]+\*?([^[:space:]]+)$ ]] ||
    [[ "${BASH_REMATCH[2]}" != "$tarball" ]]; then
    echo "Invalid checksum sidecar for $tarball" >&2
    exit 1
fi
expected_checksum="${BASH_REMATCH[1],,}"
actual_checksum="$("${checksum_cmd[@]}" "$tmpdir/$tarball" | cut -d ' ' -f1)"
if [[ "$actual_checksum" != "$expected_checksum" ]]; then
    echo "Checksum verification failed for $tarball" >&2
    exit 1
fi
tar xzf "$tmpdir/$tarball" -C "$tmpdir"
binary="$tmpdir/groky-${tag}-${target}/groky"
[[ -f "$binary" ]] || { echo "Release tarball lacks groky binary" >&2; exit 1; }
mkdir -p "$download_dir" "$bin_dir"
install -m 755 "$binary" "$download_dir/groky-${target}"
ln -sf "$download_dir/groky-${target}" "$bin_dir/groky"
ln -sf "$download_dir/groky-${target}" "$bin_dir/agent"
echo "Installed groky ${release_tag} to $bin_dir/groky" >&2
