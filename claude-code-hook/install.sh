#!/usr/bin/env bash
# SHIFT — Claude Code integration installer
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/alohaninja/shift/main/claude-code-hook/install.sh | bash
#   # or
#   ./claude-code-hook/install.sh
set -euo pipefail

CLAUDE_DIR="${HOME}/.claude"
SETTINGS_FILE="${CLAUDE_DIR}/settings.json"
PORT=8787

echo "SHIFT — Claude Code Integration"
echo "================================"
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

if ! command -v claude &>/dev/null && [ ! -d "${CLAUDE_DIR}" ]; then
  echo "✗ Claude Code not detected"
  echo
  echo "Install Claude Code first: https://claude.ai/code"
  exit 1
fi
echo "✓ Claude Code detected"
echo

# Create ~/.claude if needed
mkdir -p "${CLAUDE_DIR}"

# Update settings.json
if [ -f "${SETTINGS_FILE}" ]; then
  echo "Updating ${SETTINGS_FILE}..."
  # Use shift-ai's built-in setup for JSON manipulation
  # Fall back to simple approach if jq is not available
  if command -v jq &>/dev/null; then
    tmp=$(mktemp)
    jq '.env["ANTHROPIC_BASE_URL"] = "http://localhost:'"${PORT}"'"' "${SETTINGS_FILE}" > "${tmp}" && mv "${tmp}" "${SETTINGS_FILE}"
  else
    echo "  (jq not found — please manually add to ${SETTINGS_FILE}):"
    echo '  "env": { "ANTHROPIC_BASE_URL": "http://localhost:'"${PORT}"'" }'
  fi
else
  echo "Creating ${SETTINGS_FILE}..."
  cat > "${SETTINGS_FILE}" <<EOF
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://localhost:${PORT}"
  }
}
EOF
fi
echo "✓ Claude Code configured to use SHIFT proxy"

# Start proxy
echo
echo "Starting SHIFT proxy..."
shift-ai proxy ensure --quiet && echo "✓ Proxy running on port ${PORT}" || echo "✗ Failed to start proxy"

echo
echo "Done! Claude Code will now route API traffic through SHIFT."
echo "Run 'shift-ai gain' to see cumulative token savings."
