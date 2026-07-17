#!/usr/bin/env bash
# Installer for groky. Downloads a release tarball from GitHub, verifies its
# checksum, and installs the binary to ~/.local/bin (no sudo, ever).
#
# Environment overrides:
#   GROKY_VERSION        release tag to install, e.g. "v0.1.0" (default: latest)
#   GROKY_INSTALL_DIR    install destination (default: $HOME/.local/bin)
#   GROKY_DOWNLOAD_BASE  base URL for the tarball (default: GitHub releases)
set -euo pipefail

REPO="amanverasia/groky"

die() {
    echo "error: $*" >&2
    exit 1
}

# --- Requirements -----------------------------------------------------------
command -v curl >/dev/null || die "curl is required but not found"
command -v tar >/dev/null || die "tar is required but not found"

SHA256_TOOL=""
if command -v sha256sum >/dev/null; then
    SHA256_TOOL="sha256sum"
elif command -v shasum >/dev/null; then
    SHA256_TOOL="shasum -a 256"
else
    die "sha256sum or shasum is required but neither was found"
fi

# --- Platform detection -----------------------------------------------------
os=$(uname -s)
case "$os" in
    Linux) os_triple="unknown-linux-gnu" ;;
    Darwin) os_triple="apple-darwin" ;;
    *) die "unsupported OS: $os (groky supports Linux and macOS)" ;;
esac

arch=$(uname -m)
case "$arch" in
    x86_64 | amd64) arch="x86_64" ;;
    aarch64 | arm64) arch="aarch64" ;;
    *) die "unsupported architecture: $arch (groky supports x86_64 and aarch64)" ;;
esac

target="${arch}-${os_triple}"

# --- Resolve version --------------------------------------------------------
tag="${GROKY_VERSION:-}"
if [ -z "$tag" ]; then
    api_url="https://api.github.com/repos/$REPO/releases/latest"
    tag=$(curl -fsSL "$api_url" 2>/dev/null |
        grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' |
        head -n1 | sed 's/.*"\(v[^"]*\)"$/\1/') || true
    [ -n "$tag" ] || die "could not determine the latest release from $api_url;
set GROKY_VERSION (e.g. GROKY_VERSION=v0.1.0) and retry"
fi
version="${tag#v}"

# --- Download ----------------------------------------------------------------
base="${GROKY_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$tag}"
tarball="groky-${version}-${target}.tar.gz"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

echo "Downloading $base/$tarball"
curl -fsSL -o "$tmpdir/$tarball" "$base/$tarball" ||
    die "download failed: $base/$tarball (does release $tag include $target?)"
curl -fsSL -o "$tmpdir/$tarball.sha256" "$base/$tarball.sha256" ||
    die "download failed: $base/$tarball.sha256"

# --- Verify checksum ---------------------------------------------------------
# The .sha256 file references the bare tarball filename, so verify in tmpdir.
if ! (cd "$tmpdir" && $SHA256_TOOL -c "$tarball.sha256" >/dev/null 2>&1); then
    rm -f "$tmpdir/$tarball"
    die "checksum verification failed for $tarball; the download was discarded"
fi
echo "Checksum verified."

# --- Install -----------------------------------------------------------------
tar xzf "$tmpdir/$tarball" -C "$tmpdir"
binary="$tmpdir/groky-${version}-${target}/groky"
[ -f "$binary" ] || die "tarball did not contain the expected groky binary"

install_dir="${GROKY_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
install -m 755 "$binary" "$install_dir/groky"

echo "Installed groky $tag to $install_dir/groky"
"$install_dir/groky" --version || true

# --- PATH hint ---------------------------------------------------------------
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *)
        echo
        echo "Note: $install_dir is not in your PATH. Add it with:"
        echo "  export PATH=\"$install_dir:\$PATH\""
        echo "(append that line to your shell profile, e.g. ~/.bashrc or ~/.zshrc)"
        ;;
esac
