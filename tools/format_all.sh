#!/bin/bash


set -e

WORKSPACE_ROOT="$(dirname "$(readlink -f "$0")")/.."

EXCLUDED_CRATES=$(python3 - "$WORKSPACE_ROOT/Cargo.toml" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as manifest_file:
    manifest = tomllib.load(manifest_file)

for excluded_crate in manifest.get("workspace", {}).get("exclude", []):
    print(excluded_crate)
PY
)

CHECK_MODE=false

if [ "$#" -eq 1 ]; then
    if [ "$1" == "--check" ]; then
        CHECK_MODE=true
    else
        echo "Error: Invalid argument. Only '--check' is allowed."
        exit 1
    fi
elif [ "$#" -gt 1 ]; then
    echo "Error: Too many arguments. Only '--check' is allowed."
    exit 1
fi

cd $WORKSPACE_ROOT
if [ "$CHECK_MODE" = true ]; then
    cargo fmt --check
else
    cargo fmt
fi

KERNEL_DEMO_FILE="$WORKSPACE_ROOT/osdk/tests/examples_in_book/write_a_kernel_in_100_lines_templates/lib.rs"
if [ "$CHECK_MODE" = true ]; then
    rustfmt --check $KERNEL_DEMO_FILE
else
    rustfmt $KERNEL_DEMO_FILE
fi

for CRATE in $EXCLUDED_CRATES; do
    CRATE_DIR="$WORKSPACE_ROOT/$CRATE"

    case "$CRATE" in
        *cargo-component*)
            continue
            ;;
        *target/osdk/base*)
            continue
            ;;
        *target/osdk/test-base*)
            continue
            ;;
    esac

    if [ -d "$CRATE_DIR" ]; then
        if [ "$CHECK_MODE" = true ]; then
            (cd "$CRATE_DIR" && cargo fmt --check)
        else
            (cd "$CRATE_DIR" && cargo fmt)
        fi
    else
        echo "Directory for crate $CRATE does not exist"
    fi
done
