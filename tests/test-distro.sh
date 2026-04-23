#!/usr/bin/env bash
#
# Cross-distro integration test for SHIFT
# Runs inside Docker containers to verify build + test on multiple Linux distros
#
set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

FAILURES=0

info()    { echo -e "${BLUE}[INFO]${NC} $*"; }
pass()    { echo -e "${GREEN}[PASS]${NC} $*"; }
fail()    { echo -e "${RED}[FAIL]${NC} $*"; FAILURES=$((FAILURES + 1)); }
section() { echo ""; echo -e "${YELLOW}━━━ $* ━━━${NC}"; }

detect_os() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        echo "$NAME $VERSION_ID"
    else
        echo "unknown"
    fi
}

# ── Environment ───────────────────────────────────────────────
section "Environment"
info "OS:   $(detect_os)"
info "User: $(whoami)"
info "Rust: $(rustc --version)"
info "Cargo: $(cargo --version)"

# ── Build ─────────────────────────────────────────────────────
section "Build (release)"
if cargo build --release --workspace 2>&1; then
    pass "cargo build --release --workspace"
else
    fail "cargo build --release --workspace"
    exit 1
fi

# Verify binary exists
if [ -f target/release/shift-ai ]; then
    SIZE=$(stat -c%s target/release/shift-ai 2>/dev/null || stat -f%z target/release/shift-ai)
    pass "Binary exists: target/release/shift-ai ($(echo "$SIZE" | awk '{printf "%.1f MB", $1/1048576}'))"
else
    fail "Binary not found: target/release/shift-ai"
    exit 1
fi

# ── Tests ─────────────────────────────────────────────────────
section "Tests"
if cargo test --workspace 2>&1 | tee /tmp/test-output.txt; then
    RESULTS=$(grep "^test result:" /tmp/test-output.txt || echo "no summary")
    pass "cargo test --workspace"
    info "Results: $RESULTS"
else
    fail "cargo test --workspace"
    grep "^test result:" /tmp/test-output.txt || true
fi

# ── CLI smoke tests ───────────────────────────────────────────
section "CLI Smoke Tests"

# --help
if ./target/release/shift-ai --help >/dev/null 2>&1; then
    pass "shift --help"
else
    fail "shift --help"
fi

# --version
if ./target/release/shift-ai --version >/dev/null 2>&1; then
    pass "shift --version"
else
    fail "shift --version"
fi

# Dry-run with OpenAI fixture
if ./target/release/shift-ai --dry-run < tests/fixtures/openai_request.json >/dev/null 2>&1; then
    pass "shift --dry-run (OpenAI fixture)"
else
    fail "shift --dry-run (OpenAI fixture)"
fi

# Dry-run with Anthropic fixture
if ./target/release/shift-ai --dry-run < tests/fixtures/anthropic_request.json >/dev/null 2>&1; then
    pass "shift --dry-run (Anthropic fixture)"
else
    fail "shift --dry-run (Anthropic fixture)"
fi

# Report output mode
if ./target/release/shift-ai -o report < tests/fixtures/openai_request.json >/dev/null 2>&1; then
    pass "shift -o report (OpenAI fixture)"
else
    fail "shift -o report (OpenAI fixture)"
fi

# ── Binary dependencies ──────────────────────────────────────
section "Binary Dependencies"
if command -v ldd >/dev/null 2>&1; then
    info "Linked libraries:"
    ldd target/release/shift-ai | head -20
else
    info "ldd not available, skipping"
fi

# ── Summary ───────────────────────────────────────────────────
section "Summary"
if [ "$FAILURES" -eq 0 ]; then
    echo -e "${GREEN}All tests passed on $(detect_os)${NC}"
    exit 0
else
    echo -e "${RED}${FAILURES} test(s) failed on $(detect_os)${NC}"
    exit 1
fi
