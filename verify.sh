#!/usr/bin/env bash
# verify.sh: local pre-push gate.
# Builds all four contracts via cargo-near (deployable WASM), hashes them,
# runs clippy strict and fmt check, then the twelve near-workspaces integration tests.
# Exit 0 only if every step is green.

set -euo pipefail
cd "$(dirname "$0")"

CRATES=(sub-account-locker resale-locker tla-manager tla-registry)

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy (strict, wasm target)"
cargo clippy --target wasm32-unknown-unknown --release --workspace -- -D warnings

echo "==> cargo near build (non-reproducible-wasm --no-abi) per crate"
for c in "${CRATES[@]}"; do
  (cd "contracts/$c" && cargo near build non-reproducible-wasm --no-abi >/dev/null)
done

echo "==> deployable artifact hashes"
for c in "${CRATES[@]}"; do
  artifact="target/near/${c//-/_}/${c//-/_}.wasm"
  printf "%7d  %s  %s\n" "$(stat -c%s "$artifact")" "$(sha256sum "$artifact" | awk '{print $1}')" "$artifact"
done

echo "==> embedded-bundle hash check"
embedded_locker=$(sha256sum contracts/tla-manager/res/sub_account_locker.wasm | awk '{print $1}')
current_locker=$(sha256sum target/near/sub_account_locker/sub_account_locker.wasm | awk '{print $1}')
embedded_resale=$(sha256sum contracts/tla-registry/res/resale_locker.wasm | awk '{print $1}')
current_resale=$(sha256sum target/near/resale_locker/resale_locker.wasm | awk '{print $1}')
echo "  manager   embeds sub_account_locker.wasm = $embedded_locker"
echo "  current   sub_account_locker build       = $current_locker"
echo "  registry  embeds resale_locker.wasm      = $embedded_resale"
echo "  current   resale_locker build            = $current_resale"
if [ "$embedded_locker" != "$current_locker" ]; then
  echo "  NOTE: manager bundles an earlier sub_account_locker build; this is expected"
  echo "        for the layered audit freeze. See BUILD.md 'Layered freeze caveat'."
fi
if [ "$embedded_resale" != "$current_resale" ]; then
  echo "  NOTE: registry bundles an earlier resale_locker build; same caveat."
fi

echo "==> integration tests (12 scenarios)"
cargo test -p tla-registry --test integration -- --test-threads=1

echo "==> verify.sh OK"
