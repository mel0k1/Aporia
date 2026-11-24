#!/bin/bash


set -e


SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/../..

print_help() {
    echo "Usage: $0 [--dry-run | --token REGISTRY_TOKEN]"
    echo ""
    echo "Options:"
    echo "  --dry-run:               Execute the publish check without actually publishing it."
    echo "  --token REGISTRY_TOKEN:  The token to authenticate with crates.io."
}

DRY_RUN=""
TOKEN=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run)
            DRY_RUN="true"
            shift
            ;;
        --token)
            TOKEN="$2"
            shift 2
            ;;
        *)
            echo "Error: Invalid parameter: $1"
            print_help
            exit 1
            ;;
    esac
done

do_publish_for() {
    pushd $ASTER_SRC_DIR/$1

    ADDITIONAL_ARGS="${@:2}"
    RF="$RUSTFLAGS --check-cfg cfg(ktest)"

    if [ -n "$DRY_RUN" ]; then
        RUSTFLAGS=$RF cargo publish --dry-run --allow-dirty $ADDITIONAL_ARGS
        RUSTFLAGS=$RF cargo doc $ADDITIONAL_ARGS
    else
        RUSTFLAGS=$RF cargo publish --token $TOKEN $ADDITIONAL_ARGS
    fi

    popd
}

do_publish_for tools/sctrace
