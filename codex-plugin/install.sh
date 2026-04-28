#!/usr/bin/env bash
# SHIFT — Codex CLI integration installer
#
# Writes openai_base_url to ~/.codex/config.toml so Codex CLI routes
# API requests through the SHIFT proxy.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/alohaninja/shift/main/codex-plugin/install.sh | bash
#   # or
#   ./codex-plugin/install.sh
set -euo pipefail

PORT=8787
CODEX_CONFIG="${HOME}/.codex/config.toml"

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
  echo "⚠ codex not found on PATH (will still configure config.toml)"
fi
echo

# Configure ~/.codex/config.toml
mkdir -p "${HOME}/.codex"

KEY="openai_base_url"
VALUE="http://localhost:${PORT}"

if [ -f "${CODEX_CONFIG}" ] && grep -q "${KEY}" "${CODEX_CONFIG}" 2>/dev/null; then
  echo "✓ ${KEY} already set in ${CODEX_CONFIG}"
else
  echo "Adding ${KEY} to ${CODEX_CONFIG}..."
  echo "${KEY} = \"${VALUE}\"" >> "${CODEX_CONFIG}"
  echo "✓ Configured"
fi

# Start proxy
echo
echo "Starting SHIFT proxy..."
shift-ai proxy ensure --quiet && echo "✓ Proxy running on port ${PORT}" || echo "✗ Failed to start proxy"

echo
echo "Done! Codex CLI will now route API traffic through SHIFT."
echo "Run 'shift-ai gain' to see cumulative token savings."
