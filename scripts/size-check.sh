#!/usr/bin/env bash
# Size regression gate for the uxuy-router program binary.
# Baseline lives in scripts/size-baseline.txt — update it deliberately
# (toolchain bumps, adapter adds/removes), never silently.
set -euo pipefail
cd "$(dirname "$0")/.."

# canonical toolchain (installed 2026-10-20; see ~/.config/uxuy/sbf-env.sh)
source "$HOME/.config/uxuy/sbf-env.sh" 2>/dev/null \
  || export PATH="$HOME/.cache/solana/platform-tools-latest/rust/bin:$PATH"

rm -rf target/sbpfv3-solana-solana
cargo build -p uxuy-router --release --target sbpfv3-solana-solana

size=$(stat -f%z target/sbpfv3-solana-solana/release/uxuy_router.so)
base=$(grep -m1 -oE '^[0-9]+' scripts/size-baseline.txt)
ceiling=1017728

echo "toolchain: $(rustc --version)"
echo "size: ${size}  baseline: ${base}  delta: $((size - base))"
if [ "$size" -gt "$ceiling" ]; then
  echo "OVER the BPF size ceiling ($ceiling) — deploy needs --max-len or a program-id migration"
  exit 1
fi
