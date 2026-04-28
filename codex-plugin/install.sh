#!/usr/bin/env bash
# SHIFT — Codex CLI integration installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/alohaninja/shift/main/codex-plugin/install.sh | bash
#   # or
#   ./codex-plugin/install.sh
set -euo pipefail

PORT=8787
SHELL_RC=""

echo "SHIFT — Codex CLI Integration"
echo "=============================="
echo

# Check prerequisites
if ! command -v shift-ai &>/dev/null; then
  echo "✗ shift-ai not found"
  echo
  echo "Install shift-ai first:"
  echo "  brew install alohaninja/shift/shift-ai"
  echo "  # or: cargo install shift-preflight-cli"
  exit 1
fi
echo "✓ shift-ai $(shift-ai --version 2>/dev/null | head -1 || echo 'found')"

if ! command -v codex &>/dev/null; then
  echo "⚠ codex not found on PATH (will still configure env var)"
fi
echo

# Detect shell profile
if [ -n "${ZSH_VERSION:-}" ] || [ "${SHELL:-}" = "$(command -v zsh)" ]; then
  SHELL_RC="${HOME}/.zshrc"
elif [ -n "${BASH_VERSION:-}" ] || [ "${SHELL:-}" = "$(command -v bash)" ]; then
  SHELL_RC="${HOME}/.bashrc"
else
  SHELL_RC="${HOME}/.profile"
fi

# Check if already configured
MARKER='shift-ai env codex'
if grep -q "${MARKER}" "${SHELL_RC}" 2>/dev/null; then
  echo "✓ Already configured in ${SHELL_RC}"
else
  echo "Adding SHIFT env to ${SHELL_RC}..."
  {
    echo
    echo '# SHIFT — Codex CLI image optimization proxy'
    echo 'eval "$(shift-ai env codex)"'
  } >> "${SHELL_RC}"
  echo "✓ Added to ${SHELL_RC}"
fi

# Start proxy
echo
echo "Starting SHIFT proxy..."
shift-ai proxy ensure --quiet && echo "✓ Proxy running on port ${PORT}" || echo "✗ Failed to start proxy"

echo
echo "Done! Open a new terminal for changes to take effect."
echo "Run 'shift-ai gain' to see cumulative token savings."
