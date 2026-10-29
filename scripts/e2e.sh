#!/bin/sh
# End to end, as a new user would: install grh with install.sh from this
# build, install every part in a fixture repository with the real part
# releases, use one, uninstall everything, and check HOME is as it was.
#
#   scripts/e2e.sh <path to the built grh binary>
#
# Everything happens under a scratch HOME. The tool state dirs the parts keep
# after their own uninstall (relais's state, amont-agent's journal) are the
# only leftovers allowed.
set -eu

built=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
here=$(cd "$(dirname "$0")/.." && pwd)
version=$(grep -m1 '^version = ' "$here/Cargo.toml" | cut -d'"' -f2)

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM
export HOME="$scratch/home"
export XDG_STATE_HOME="$HOME/.local/state" XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache"
export CLAUDE_CONFIG_DIR="$HOME/.claude"
export RELAIS_STATE_DIR="$HOME/.local/state/relais" RELAIS_CONFIG_DIR="$HOME/.config/relais"
export GRH_BIN_DIR="$HOME/.local/bin"
mkdir -p "$HOME" "$GRH_BIN_DIR"
export PATH="$GRH_BIN_DIR:/usr/bin:/bin:/usr/sbin:/sbin"
git config --global user.email e2e@example.invalid
git config --global user.name e2e

# This build, packaged the way the release workflow packages it.
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux-aarch64) target=aarch64-unknown-linux-gnu ;;
    Darwin-arm64) target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
esac
dist="$scratch/dist"
mkdir -p "$dist/grh-$version-$target"
cp "$built" "$dist/grh-$version-$target/grh"
(cd "$dist" && tar czf "grh-$version-$target.tar.gz" "grh-$version-$target")
if command -v sha256sum > /dev/null 2>&1; then
    (cd "$dist" && sha256sum "grh-$version-$target.tar.gz" > SHA256SUMS)
else
    (cd "$dist" && shasum -a 256 "grh-$version-$target.tar.gz" > SHA256SUMS)
fi

before="$scratch/before.txt"
(cd "$HOME" && find . -type f | sort) > "$before"

step() { printf '\n== %s\n' "$1"; }

step "install.sh, from this build"
GRH_BASE_URL="file://$dist" GRH_VERSION="v$version" sh "$here/install/install.sh"
grh --version | grep -q "$version"

step "grh install full workflow --repo, in a fixture repository"
repo="$scratch/repo"
git init -q "$repo"
(cd "$repo" && git commit -q --allow-empty -m "chore: seed")
(cd "$repo" && grh install full workflow --repo --yes)

step "grh status --json is one JSON document"
grh status --json > "$scratch/status.json"
python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$scratch/status.json"

step "a part answers through grh"
grh decisions --version

step "grh uninstall, bare, as the site prints it"
(cd "$repo" && grh uninstall --yes)

step "HOME is as it was, apart from grh and the tools' own state"
(cd "$HOME" && find . -type f | sort) > "$scratch/after.txt"
leftover=$(comm -13 "$before" "$scratch/after.txt" \
    | grep -v -e '^\./\.local/state/grh/' -e '^\./\.local/state/relais/' \
              -e '^\./\.claude/amont-agent/' -e '^\./\.local/bin/grh$' \
              -e '^\./\.cache/' || true)
if [ -n "$leftover" ]; then
    echo "left behind after uninstall:"
    printf '%s\n' "$leftover"
    exit 1
fi
echo "nothing left behind"
