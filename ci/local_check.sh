#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo-binstall >/dev/null 2>&1; then
  # Bootstrap from the prebuilt release binary: nothing is compiled, so a
  # cargo-binstall dependency raising its MSRV above the pinned toolchain
  # cannot break this step (cargo-binstall 1.22.0 did exactly that).
  binstall_ok=0
  for attempt in 1 2 3; do
    # `curl | bash` would hide a download failure: the pipeline reports
    # bash's status, and bash succeeds on empty input. Download, then run.
    binstall_installer="$(mktemp)"
    if curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh -o "$binstall_installer" && bash "$binstall_installer"; then
      hash -r
      if command -v cargo-binstall >/dev/null 2>&1; then binstall_ok=1; fi
    fi
    rm -f "$binstall_installer"
    if [ "$binstall_ok" -eq 1 ]; then break; fi
    sleep $((attempt * 5))
  done
  if [ "$binstall_ok" -ne 1 ]; then
    # Last release whose bundled lockfile still builds on 1.95.0.
    cargo install cargo-binstall --locked --version 1.21.1
  fi
fi

cargo binstall -y greentic-flow --force
cargo binstall -y greentic-pack --force

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

if command -v greentic-integration-tester >/dev/null 2>&1; then
  greentic-integration-tester run --gtest tests/gtests/smoke --artifacts-dir artifacts/gtests --workdir .
else
  echo "greentic-integration-tester not found; skipping gtests."
fi
