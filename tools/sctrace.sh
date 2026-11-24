#!/bin/bash


set -e

SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
ASTERINAS_ROOT="$(dirname "$SCRIPT_DIR")"

cargo run -q --manifest-path "$ASTERINAS_ROOT/tools/sctrace/Cargo.toml" -- "$@"
