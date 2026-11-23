#!/usr/bin/env bash
# Builds the browser version of the Spheradian demo into docs/demo/pkg/.
#
# Requirements:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --locked --version <the wasm-bindgen version in Cargo.lock>
#   (optional) wasm-opt from binaryen, to shrink the output further
#
# Serve docs/ with any static file server to try it locally, for example:
#   python3 -m http.server -d docs 8000   # then open http://localhost:8000/demo/
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

out_dir="docs/demo/pkg"
target="wasm32-unknown-unknown"
# Pass the target directory to Cargo explicitly, so the artifact is read from where it
# was written even when CARGO_TARGET_DIR or `build.target-dir` is set.
target_dir="${CARGO_TARGET_DIR:-$root/target}"
wasm="$target_dir/$target/release/spheradian.wasm"

# The output directory is deleted before wasm-bindgen runs, so Cargo must not write
# its artifacts inside it.
mkdir -p "$out_dir" "$target_dir"
out_abs="$(cd "$out_dir" && pwd -P)"
target_abs="$(cd "$target_dir" && pwd -P)"
if [[ "$target_abs/" == "$out_abs/"* ]]; then
  echo "The Cargo target directory $target_dir is inside $out_dir, which this script deletes." >&2
  echo "Set CARGO_TARGET_DIR to a directory outside $out_dir." >&2
  exit 1
fi

lock_version="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)"
cli_version="$(wasm-bindgen --version | awk '{ print $2 }')"
if [[ "$lock_version" != "$cli_version" ]]; then
  echo "wasm-bindgen CLI $cli_version does not match wasm-bindgen $lock_version in Cargo.lock." >&2
  echo "Install the matching CLI: cargo install wasm-bindgen-cli --locked --version $lock_version" >&2
  exit 1
fi

# The native release profile strips all symbols, which also drops the custom section
# wasm-bindgen reads to detect enabled wasm features. Keep it here; wasm-bindgen
# removes the name and producers sections itself below.
CARGO_PROFILE_RELEASE_STRIP=debuginfo cargo build --release --locked --target "$target" --target-dir "$target_dir"

rm -rf "$out_dir"
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
  --out-dir "$out_dir" "$wasm"

if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    -o "$out_dir/spheradian_bg.wasm" "$out_dir/spheradian_bg.wasm"
else
  echo "wasm-opt not found; skipping size optimization." >&2
fi

ls -lh "$out_dir"
