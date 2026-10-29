#!/bin/sh
# Install the grh binary. Nothing else.
#
#   curl -fsSL https://raw.githubusercontent.com/greenharness/grh/main/install/install.sh | sh
#
# It downloads a checksum-verified grh and puts it on your PATH. It installs
# no part and arms no repository: `grh install <parts>` does that, after
# showing you what it will change.
#
# POSIX sh, no bashisms: this has to run on a minimal CI image with no bash.
#
#   GRH_BIN_DIR    where grh goes (default ~/.local/bin)
#   GRH_VERSION    a release such as v0.1.0 (default: the latest)
#   GRH_BASE_URL   the directory holding the tarball and SHA256SUMS, instead
#                  of the GitHub release (used by CI to test its own build)
set -eu

REPO="greenharness/grh"
BIN_DIR="${GRH_BIN_DIR:-$HOME/.local/bin}"
VERSION="${GRH_VERSION:-latest}"

RED='\033[31m'; GREEN='\033[32m'; YELLOW='\033[33m'; OFF='\033[0m'
if [ -n "${NO_COLOR:-}" ] || [ ! -t 1 ]; then RED=''; GREEN=''; YELLOW=''; OFF=''; fi

say()  { printf '  %s\n' "$1"; }
ok()   { printf "  ${GREEN}✓${OFF} %s\n" "$1"; }
warn() { printf "  ${YELLOW}!${OFF} %s\n" "$1"; }
die()  { printf "  ${RED}✗${OFF} %s\n" "$1" >&2; exit 1; }

need() { command -v "$1" > /dev/null 2>&1 || die "$1 is required and was not found"; }

need uname
need tar

if command -v curl > /dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    fetch_to() { curl -fsSL "$1" -o "$2"; }
elif command -v wget > /dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    fetch_to() { wget -qO "$2" "$1"; }
else
    die "neither curl nor wget is available"
fi

# A sha256 tool is required, not optional: grh is the binary that then
# fetches and runs every other part, so an unverified grh is worse than none.
if command -v sha256sum > /dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum > /dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    die "no sha256 tool (sha256sum or shasum) found: refusing to install an unverified grh"
fi

target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux)
            if ldd --version 2>&1 | grep -qi musl; then libc=musl; else libc=gnu; fi
            case "$arch" in
                x86_64|amd64)  echo "x86_64-unknown-linux-$libc" ;;
                aarch64|arm64) [ "$libc" = "musl" ] && die "no aarch64 musl build yet: cargo install grh"
                               echo "aarch64-unknown-linux-gnu" ;;
                *) die "unsupported architecture: $arch (cargo install grh)" ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                x86_64) echo "x86_64-apple-darwin" ;;
                arm64)  echo "aarch64-apple-darwin" ;;
                *) die "unsupported architecture: $arch" ;;
            esac
            ;;
        *) die "unsupported OS: $os (cargo install grh)" ;;
    esac
}

resolve_version() {
    if [ "$VERSION" != "latest" ]; then
        echo "${VERSION#v}"
        return
    fi
    [ -z "${GRH_BASE_URL:-}" ] || die "GRH_BASE_URL needs GRH_VERSION"
    tag=$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$tag" ] || die "could not determine the latest release (rate limited? set GRH_VERSION=vX.Y.Z)"
    echo "${tag#v}"
}

main() {
    printf '\n  grh installer\n\n'

    t=$(target)
    v=$(resolve_version)
    name="grh-${v}-${t}"
    base="${GRH_BASE_URL:-https://github.com/$REPO/releases/download/v${v}}"

    say "version:  $v"
    say "platform: $t"
    say "into:     $BIN_DIR"
    printf '\n'

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM

    say "downloading…"
    fetch_to "$base/${name}.tar.gz" "$tmp/${name}.tar.gz" \
        || die "download failed: $base/${name}.tar.gz"
    fetch_to "$base/SHA256SUMS" "$tmp/SHA256SUMS" \
        || die "no SHA256SUMS at $base: refusing to install an unverified grh"

    got=$(sha256 "$tmp/${name}.tar.gz")
    want=$(grep " \(\./\)\{0,1\}${name}.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1 | head -n 1)
    [ -n "$want" ] || die "SHA256SUMS has no entry for ${name}.tar.gz"
    [ "$got" = "$want" ] || die "checksum mismatch, refusing to install
    expected $want
    got      $got"
    ok "checksum verified"

    tar xzf "$tmp/${name}.tar.gz" -C "$tmp"
    [ -f "$tmp/$name/grh" ] || die "$name archive holds no grh binary"
    mkdir -p "$BIN_DIR"
    # A temporary name, then rename: replacing a running binary in place
    # fails on some platforms, and rename is atomic.
    cp "$tmp/$name/grh" "$BIN_DIR/.grh.new"
    chmod 755 "$BIN_DIR/.grh.new"
    mv "$BIN_DIR/.grh.new" "$BIN_DIR/grh"
    ok "installed $BIN_DIR/grh"

    printf '\n'
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) warn "$BIN_DIR is not on your PATH: add it to run grh by name" ;;
    esac
    printf '  Nothing else is installed yet. Next:\n\n'
    printf '    grh audit                  # what fits this repository\n'
    printf '    grh install solo           # gate and guardrails\n'
    printf '    grh install team           # + decisions and conform\n\n'
}

main "$@"
