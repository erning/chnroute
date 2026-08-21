#!/bin/sh

set -eu

LC_ALL=C
export LC_ALL

DIST_BRANCH=dist
REMOTE_NAME=origin
PUSH=false

usage() {
    printf '%s\n' \
        'Usage: scripts/publish-dist.sh [--push]' \
        '' \
        'Generate route tables and publish them to the local dist branch.' \
        '' \
        'Options:' \
        '  --push      Push the resulting dist branch to origin' \
        '  -h, --help  Print help'
}

die() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --push)
            PUSH=true
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
    shift
done

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel 2>/dev/null) ||
    die 'script must be run from a Git worktree'
cd "$REPO_ROOT"

[ -f Cargo.toml ] || die 'Cargo.toml is missing from the source worktree'

SOURCE_COMMIT=$(git rev-parse --verify HEAD 2>/dev/null) ||
    die 'source worktree has no commit'
WORKTREE_STATUS=$(git status --porcelain --untracked-files=normal)
[ -z "$WORKTREE_STATUS" ] ||
    die 'source worktree must be clean before publishing'

if [ "$PUSH" = true ]; then
    git remote get-url "$REMOTE_NAME" >/dev/null 2>&1 ||
        die "Git remote is not configured: $REMOTE_NAME"
fi

printf '%s\n' 'running offline tests'
cargo test --locked --offline

printf '%s\n' 'generating distribution'
cargo run --locked --offline --release -- generate

DIST_DIR=$REPO_ROOT/dist
[ -d "$DIST_DIR" ] && [ ! -L "$DIST_DIR" ] ||
    die 'generate did not create a regular dist directory'
[ -f "$DIST_DIR/manifest.json" ] ||
    die 'generated distribution is missing manifest.json'

PUBLISH_PARENT=$(mktemp -d "${TMPDIR:-/tmp}/chnroute-publish.XXXXXX") ||
    die 'failed to create temporary publication directory'
PUBLISH_TREE=$PUBLISH_PARENT/worktree

cleanup() {
    if [ -e "$PUBLISH_TREE/.git" ]; then
        git worktree remove --force "$PUBLISH_TREE" >/dev/null 2>&1 || true
    fi
    rmdir "$PUBLISH_PARENT" >/dev/null 2>&1 || true
}

trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

if git show-ref --verify --quiet "refs/heads/$DIST_BRANCH"; then
    git worktree add --quiet "$PUBLISH_TREE" "$DIST_BRANCH"
elif git show-ref --verify --quiet "refs/remotes/$REMOTE_NAME/$DIST_BRANCH"; then
    git branch --no-track "$DIST_BRANCH" "refs/remotes/$REMOTE_NAME/$DIST_BRANCH" >/dev/null
    git worktree add --quiet "$PUBLISH_TREE" "$DIST_BRANCH"
else
    git worktree add --quiet --detach "$PUBLISH_TREE" "$SOURCE_COMMIT"
    git -C "$PUBLISH_TREE" switch --quiet --orphan "$DIST_BRANCH"
fi

git -C "$PUBLISH_TREE" rm -r --quiet --ignore-unmatch .
cp -R "$DIST_DIR/." "$PUBLISH_TREE/"
git -C "$PUBLISH_TREE" add -A
git -C "$PUBLISH_TREE" diff --cached --check

DIST_CHANGED=false
if git -C "$PUBLISH_TREE" diff --cached --quiet; then
    printf '%s\n' 'distribution is already current'
else
    git -C "$PUBLISH_TREE" commit \
        -m 'chore(dist): publish route tables' \
        -m "Generated-From: $SOURCE_COMMIT" \
        -m 'Upstream source details and artifact hashes are recorded in manifest.json.'
    DIST_CHANGED=true
fi

DIST_COMMIT=$(git -C "$PUBLISH_TREE" rev-parse HEAD)
printf 'published local dist branch at %s\n' "$DIST_COMMIT"

if [ "$PUSH" = true ] && [ "$DIST_CHANGED" = true ]; then
    git -C "$PUBLISH_TREE" push "$REMOTE_NAME" \
        "refs/heads/$DIST_BRANCH:refs/heads/$DIST_BRANCH"
    printf 'pushed dist branch to %s\n' "$REMOTE_NAME"
elif [ "$PUSH" = true ]; then
    printf '%s\n' 'distribution is unchanged; skipping push'
fi
