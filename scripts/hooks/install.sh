#!/bin/sh
# scripts/hooks/install.sh — wire this repo's git hooks. Run once after clone.
set -e
cd "$(git rev-parse --show-toplevel)"
chmod +x scripts/hooks/commit-msg scripts/hooks/pre-commit
git config core.hooksPath scripts/hooks
echo "Installed hooks from scripts/hooks (core.hooksPath set)."
