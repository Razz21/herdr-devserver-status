#!/bin/sh
# fetch-or-build.sh — herdr [[build]] step for herdr-devserver-status.
#
# Fast path: download the release package matching this source's declared
# version + platform (binary + seed frameworks/*.yml, see release.yml),
# verify SHA-256, install:
#   - binary  -> target/release/herdr-devserver-status
#   - configs -> frameworks/*.yml (repo root)
# Matched by version (from Cargo.toml), not commit — a checkout ahead of
# the matching release (unreleased merges on main) still uses it, so a
# release doesn't force new installs to compile.
#
# Fallback on any miss (missing asset, network error, checksum mismatch,
# unmapped platform, corrupt/incomplete package, missing curl/wget/tar/
# sha256 tool): build from source. No extra frameworks handling needed
# there — the git checkout already has frameworks/*.yml matching that
# source tree.
#
set -u

repo="Razz21/herdr-devserver-status"

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root="$script_dir/.."
cargo_toml="$repo_root/Cargo.toml"
out="$repo_root/target/release/herdr-devserver-status"
frameworks_out="$repo_root/frameworks"
base_url="https://github.com/$repo/releases/download"

have() { command -v "$1" >/dev/null 2>&1; }

# Source ~/.cargo/env so cargo is found even when herdr was launched
# without ~/.cargo/bin on PATH (e.g. GUI / login-less launch); the
# `[ -f ]` guard means a missing env file can't abort the build.
build_from_source() {
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    if ! have cargo; then
        echo "herdr-devserver-status needs Rust 1.97+ to build, but cargo was not found. Install Rust from https://rustup.rs then re-run: herdr plugin install $repo" >&2
        exit 1
    fi
    exec cargo build --release
}

fallback() {
    echo "herdr-devserver-status: $1 — building from source instead." >&2
    [ -n "${tmpdir:-}" ] && rm -rf "$tmpdir"
    build_from_source
}

download() {
    if have curl; then
        curl -fsSL -o "$2" "$1"
    elif have wget; then
        wget -q -O "$2" "$1"
    else
        return 127
    fi
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | awk '{print $1}'
    elif have shasum; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 127
    fi
}

# --- resolve the target triple from the platform ---------------------------
# x86_64 Linux ships musl (static, avoids glibc-version mismatches across
# distros). aarch64 Linux stays gnu — no apt-installable musl cross
# toolchain for it, unlike musl-tools for x86_64.
os=$(uname -s 2>/dev/null || echo unknown)
arch=$(uname -m 2>/dev/null || echo unknown)
triple=""
case "$os" in
    Darwin)
        case "$arch" in
            arm64|aarch64) triple="aarch64-apple-darwin" ;;
            x86_64|amd64)  triple="x86_64-apple-darwin" ;;
        esac
        ;;
    Linux)
        case "$arch" in
            x86_64|amd64)  triple="x86_64-unknown-linux-musl" ;;
            aarch64|arm64) triple="aarch64-unknown-linux-gnu" ;;
        esac
        ;;
esac
[ -n "$triple" ] || fallback "no prebuilt package for $os/$arch"
have tar || fallback "no tar available to unpack the release package"

# --- read the version this source declares ----------------------------------
version=$(grep -E '^version *= *"' "$cargo_toml" 2>/dev/null | head -n 1 | sed -E 's/^version *= *"([^"]+)".*/\1/')
[ -n "$version" ] || fallback "could not read version from $cargo_toml"

asset="herdr-devserver-status-$triple.tar.gz"
tmpdir=$(mktemp -d 2>/dev/null) || fallback "could not create a temp dir"
trap 'rm -rf "$tmpdir"' EXIT

# --- version-only match (no commit-exactness gate) --------------------------
# ahead_note is informational only: if this is a git work tree and both
# HEAD and the release's published COMMIT marker are readable, note when
# the checkout is ahead — the installed package (binary + frameworks) is
# the released v$version, while the working tree may carry newer,
# unreleased source or specs.
ahead_note=""
if have git && git -C "$repo_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    head_rev=$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || echo nohead)
    if download "$base_url/v$version/COMMIT" "$tmpdir/COMMIT" 2>/dev/null; then
        release_commit=$(tr -d '[:space:]' < "$tmpdir/COMMIT" 2>/dev/null)
        if [ -n "$release_commit" ] && [ "$head_rev" != "$release_commit" ]; then
            ahead_note=" Note: this checkout ($head_rev) is ahead of the v$version release commit ($release_commit), so newer unreleased source is not in this package."
        fi
    fi
fi

pkg_url="$base_url/v$version/$asset"
sums_url="$base_url/v$version/SHA256SUMS"
tmppkg="$tmpdir/$asset"
tmpsums="$tmpdir/SHA256SUMS"

download "$pkg_url" "$tmppkg" || fallback "prebuilt package not available for v$version ($asset)"
download "$sums_url" "$tmpsums" || fallback "checksums not available for v$version"

# sha256sum text mode separates hash/name with two spaces; binary mode
# emits ` *name`. Accept either (`[ *]`). Escape the asset name for the
# ERE — .tar.gz's dots are metacharacters otherwise.
asset_re=$(printf '%s' "$asset" | sed 's/[.[\*^$]/\\&/g')
expected=$(grep -E "^[0-9a-f]{64} [ *]$asset_re\$" "$tmpsums" 2>/dev/null | awk '{print $1}' | head -n 1)
[ -n "$expected" ] || fallback "no checksum listed for $asset"

actual=$(sha256_of "$tmppkg") || fallback "no sha-256 tool (sha256sum/shasum) available"

if [ "$actual" != "$expected" ]; then
    fallback "checksum mismatch for $asset (expected $expected, got $actual)"
fi

# --- verified: unpack and inspect before touching anything on disk --------
extract_dir="$tmpdir/extracted"
mkdir -p "$extract_dir"
tar -xzf "$tmppkg" -C "$extract_dir" || fallback "could not extract $asset"

extracted_bin="$extract_dir/herdr-devserver-status"
extracted_frameworks="$extract_dir/frameworks"

[ -f "$extracted_bin" ] || fallback "package $asset is missing the herdr-devserver-status binary"
[ -d "$extracted_frameworks" ] && [ -n "$(ls -A "$extracted_frameworks" 2>/dev/null)" ] \
    || fallback "package $asset is missing seed frameworks/*.yml"

# mv from a tmpdir on the same filesystem is a rename: destination gets a
# fresh inode, so a currently-running instance (holding the old inode
# open) is unaffected. Never `cp` onto the existing path in place — a
# same-inode overwrite can SIGKILL (exit 137) a running process on next
# launch, once ad-hoc re-signing invalidates its signature on macOS.
chmod +x "$extracted_bin"
mkdir -p "$(dirname "$out")"
mv -f "$extracted_bin" "$out" || fallback "could not install the verified binary to $out"

# Config read once at daemon startup, not held open like the binary — a
# plain remove-then-copy is safe (no rename-swap needed). Removing the old
# set first keeps frameworks/*.yml matched to exactly this release.
rm -rf "$frameworks_out"
mkdir -p "$frameworks_out"
cp "$extracted_frameworks"/*.yml "$frameworks_out/" || fallback "could not install seed frameworks to $frameworks_out"

echo "herdr-devserver-status: installed prebuilt v$version ($triple), verified SHA-256.$ahead_note"
exit 0