#!/usr/bin/env bash
set -euo pipefail

REPO="alohaninja/shift"
INSTALL_DIR="${HOME}/.local/bin"

echo "Installing shift..."

# Check dependencies
for cmd in curl tar; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "Error: $cmd is required but not installed." >&2
    exit 1
  fi
done

# Detect OS
case "$(uname -s)" in
  Linux*)  OS="unknown-linux" ;;
  Darwin*) OS="apple-darwin" ;;
  *)
    echo "Error: unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

# Detect architecture
case "$(uname -m)" in
  x86_64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "Error: unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

# Build target triple
if [ "$OS" = "unknown-linux" ]; then
  if [ "$ARCH" = "x86_64" ]; then
    TARGET="${ARCH}-unknown-linux-musl"
  else
    TARGET="${ARCH}-unknown-linux-gnu"
  fi
else
  TARGET="${ARCH}-${OS}"
fi

ARCHIVE="shift-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ARCHIVE}"

echo "Detected: ${TARGET}"
echo "Downloading: ${URL}"

# Create install directory
mkdir -p "${INSTALL_DIR}"

# Download and extract
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

curl -fSL "${URL}" -o "${TMPDIR}/${ARCHIVE}"
tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}"
install -m 755 "${TMPDIR}/shift" "${INSTALL_DIR}/shift"

echo "Installed shift to ${INSTALL_DIR}/shift"

# Verify
if command -v shift &>/dev/null; then
  shift --version
else
  echo ""
  echo "Add ${INSTALL_DIR} to your PATH:"
  echo "  export PATH=\"${INSTALL_DIR}:\${PATH}\""
fi
