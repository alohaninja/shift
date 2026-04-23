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
BASE_URL="https://github.com/${REPO}/releases/latest/download"
URL="${BASE_URL}/${ARCHIVE}"
CHECKSUMS_URL="${BASE_URL}/checksums.txt"

echo "Detected: ${TARGET}"
echo "Downloading: ${URL}"

# Create install directory
mkdir -p "${INSTALL_DIR}"

# Download and extract
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

curl -fSL "${URL}" -o "${TMPDIR}/${ARCHIVE}"

# Verify checksum if sha256sum or shasum is available
curl -fSL "${CHECKSUMS_URL}" -o "${TMPDIR}/checksums.txt" 2>/dev/null && {
  if command -v sha256sum &>/dev/null; then
    (cd "${TMPDIR}" && sha256sum -c <(grep "${ARCHIVE}" checksums.txt))
  elif command -v shasum &>/dev/null; then
    (cd "${TMPDIR}" && shasum -a 256 -c <(grep "${ARCHIVE}" checksums.txt))
  else
    echo "Warning: sha256sum/shasum not found, skipping checksum verification" >&2
  fi
} || {
  echo "Warning: could not download checksums.txt, skipping verification" >&2
}

# Extract only the expected file (defense against path traversal)
tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}" shift
install -m 755 "${TMPDIR}/shift" "${INSTALL_DIR}/shift"

echo "Installed shift to ${INSTALL_DIR}/shift"

# Verify using the full path (avoid bash builtin collision)
if "${INSTALL_DIR}/shift" --version 2>/dev/null; then
  echo "Verified installation."
else
  echo ""
  echo "Add ${INSTALL_DIR} to your PATH:"
  echo "  export PATH=\"${INSTALL_DIR}:\${PATH}\""
fi
