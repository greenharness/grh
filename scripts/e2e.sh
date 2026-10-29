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

# A fake `claude`: records every mcp and plugin call, and answers `mcp get`
# from a state file, so the Claude Code integrations really install and undo.
fake="$scratch/fake"
export CLAUDE_FAKE_LOG="$scratch/claude.log" CLAUDE_FAKE_STATE="$scratch/claude-state"
mkdir -p "$fake" "$CLAUDE_FAKE_STATE"
: > "$CLAUDE_FAKE_LOG"
cat > "$fake/claude" <<'FAKE'
#!/bin/sh
echo "$*" >> "$CLAUDE_FAKE_LOG"
group=${1:-}
action=${2:-}
[ "$#" -ge 2 ] && shift 2 || shift "$#"
# The server name is the first argument that is not an option or its value.
name=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --scope|-s|--transport|-t|--env|-e|--header|-H) shift 2 || break ;;
        --) break ;;
        -*) shift ;;
        *) name=$1; break ;;
    esac
done
case "$group $action" in
    "mcp add") touch "$CLAUDE_FAKE_STATE/mcp.$name" ;;
    "mcp remove") rm -f "$CLAUDE_FAKE_STATE/mcp.$name" ;;
    "mcp get") [ -f "$CLAUDE_FAKE_STATE/mcp.$name" ] || exit 1 ;;
esac
exit 0
FAKE
chmod +x "$fake/claude"
export PATH="$GRH_BIN_DIR:$fake:/usr/bin:/bin:/usr/sbin:/sbin"
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

grep -q '^mcp add' "$CLAUDE_FAKE_LOG" || { echo "no claude mcp add was recorded"; exit 1; }
# Keep the parts' binaries for the adopted cases below.
tools="$scratch/tools"
mkdir -p "$tools"
cp "$GRH_BIN_DIR/amont" "$GRH_BIN_DIR/amont-agent" "$tools/"

step "grh status --json is one JSON document"
grh status --json > "$scratch/status.json"
# Every part with a binary reports it installed and placed by grh.
python3 - "$scratch/status.json" <<'PY'
import json, sys
parts = json.load(open(sys.argv[1]))["parts"]
want = {"decisions", "gate", "guardrails", "router", "conform"}
bad = [p["name"] for p in parts if p["name"] in want and not (p["installed"] and p["source"] == "placed")]
if bad:
    sys.exit(f"not installed and placed: {bad}")
print("every part installed and placed")
PY

step "a part answers through grh"
grh decisions --version

step "grh uninstall, bare, as the site prints it"
(cd "$repo" && grh uninstall --yes)

step "the fixture repository is disarmed"
shims=$(find "$repo/.git/hooks" -type f ! -name '*.sample' 2> /dev/null || true)
if [ -n "$shims" ]; then
    echo "amont shims left in the fixture repository:"
    printf '%s\n' "$shims"
    exit 1
fi
if [ -e "$repo/.claude/hooks/aval-heads.sh" ]; then
    echo "aval-heads.sh left in the fixture repository"
    exit 1
fi
if ls "$CLAUDE_FAKE_STATE"/mcp.* > /dev/null 2>&1; then
    echo "mcp servers left registered:"
    ls "$CLAUDE_FAKE_STATE"
    exit 1
fi
grep -q '^mcp remove' "$CLAUDE_FAKE_LOG" || { echo "no claude mcp remove was recorded"; exit 1; }

step "HOME is as it was, apart from grh and the tools' own state"
(cd "$HOME" && find . -type f | sort) > "$scratch/after.txt"
leftover=$(comm -13 "$before" "$scratch/after.txt" \
    | grep -v -e '^\./\.local/state/grh/' -e '^\./\.local/state/relais/' \
              -e '^\./\.claude/amont-agent/' -e '^\./\.local/bin/grh$' \
              -e '^\./\.cache/' -e '^\./Library/Caches/' || true)
# amont-agent's own uninstall leaves the settings.json its install created,
# emptied to `{}`. Allowed only while empty.
# holds-until: amont-agent's `uninstall --write` removes a settings.json it
# created and left empty (follow-up after amont-agent 3.9.0).
if [ "$(cat "$CLAUDE_CONFIG_DIR/settings.json" 2> /dev/null)" = "{}" ]; then
    leftover=$(printf '%s\n' "$leftover" | grep -v '^\./\.claude/settings\.json$' || true)
fi
if [ -n "$leftover" ]; then
    echo "left behind after uninstall:"
    printf '%s\n' "$leftover"
    exit 1
fi
echo "nothing left behind"

step "an integration that was there first survives uninstall"
"$tools/amont-agent" install --write
grep -q 'amont-agent' "$CLAUDE_CONFIG_DIR/settings.json" || { echo "amont-agent wrote no hook entry"; exit 1; }
grh install guardrails --yes
grh uninstall --yes
grep -q 'amont-agent' "$CLAUDE_CONFIG_DIR/settings.json" || { echo "uninstall removed an adopted hook entry"; exit 1; }
"$tools/amont-agent" uninstall --write

step "a binary that was there first survives uninstall"
other="$scratch/other"
mkdir -p "$other"
ln -s "$tools/amont" "$other/amont"
PATH="$other:$PATH" grh install gate --yes
if [ -e "$GRH_BIN_DIR/amont" ]; then
    echo "grh placed an amont although one was on PATH"
    exit 1
fi
PATH="$other:$PATH" grh uninstall --yes
[ -L "$other/amont" ] || { echo "uninstall removed the adopted amont"; exit 1; }
echo "adopted integration and binary left alone"
