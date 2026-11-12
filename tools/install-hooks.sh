#!/bin/sh
# Points git at the checked-in hooks. Run once per clone:
#
#   sh tools/install-hooks.sh
#
# The hook lives in the repository rather than in .git/hooks so that a change to
# it is reviewable, and `core.hooksPath` is a local setting, so this never
# touches anyone else's clone. Nothing runs on a push: the whole gate is CI's
# job, and `just gate` when it is wanted locally.

set -u

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd) || exit 1
cd "$root" || exit 1

chmod +x tools/hooks/pre-commit 2>/dev/null || true

git config core.hooksPath tools/hooks || {
    printf 'install-hooks: cannot set core.hooksPath\n'
    exit 1
}

printf 'install-hooks: a commit now runs fmt, whitespace, and clippy\n'
