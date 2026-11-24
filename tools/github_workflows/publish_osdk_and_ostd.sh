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

declare -A DRY_RUN_PUBLISHED_PACKAGES=()

mark_dry_run_published() {
    DRY_RUN_PUBLISHED_PACKAGES[$1]=true
}

is_dry_run_published() {
    [ -n "${DRY_RUN_PUBLISHED_PACKAGES[$1]}" ]
}

do_publish_for() {
    pushd "$ASTER_SRC_DIR/$1"
    shift

    local ADDITIONAL_ARGS=("$@")
    local RF="$RUSTFLAGS --check-cfg cfg(ktest)"

    if [ -n "$DRY_RUN" ]; then
        local DEP_NAME DEP_PATH LOCAL_PATH_DEPS MANIFEST_PATH METADATA PACKAGE_METADATA PACKAGE_NAME
        local PATCH_ARGS=()

        MANIFEST_PATH="$(pwd -P)/Cargo.toml"
        METADATA=$(cargo metadata --format-version=1 --no-deps --manifest-path "$MANIFEST_PATH")

        PACKAGE_METADATA=$(jq --arg manifest "$MANIFEST_PATH" '
            .packages[]
            | select(.manifest_path == $manifest)
        ' <<< "$METADATA")
        if [ -z "$PACKAGE_METADATA" ]; then
            echo "Error: cargo metadata did not contain a package for manifest: $MANIFEST_PATH" >&2
            exit 1
        fi

        PACKAGE_NAME=$(jq -r '.name' <<< "$PACKAGE_METADATA")
        if [ -z "$PACKAGE_NAME" ]; then
            echo "Error: cargo metadata did not contain a package name for manifest: $MANIFEST_PATH" >&2
            exit 1
        fi

        LOCAL_PATH_DEPS=$(jq -r '
            .dependencies[]
            | select(.path != null and .source == null and .kind != "dev")
            | [.name, .path]
            | @tsv
        ' <<< "$PACKAGE_METADATA")

        if [ -n "$LOCAL_PATH_DEPS" ]; then
            while IFS=$'\t' read -r DEP_NAME DEP_PATH; do
                if is_dry_run_published "$DEP_NAME"; then
                    PATCH_ARGS+=(--config "patch.crates-io.$DEP_NAME.path=\"$DEP_PATH\"")
                fi
            done <<< "$LOCAL_PATH_DEPS"
        fi

        RUSTFLAGS=$RF cargo publish --dry-run --allow-dirty "${PATCH_ARGS[@]}" "${ADDITIONAL_ARGS[@]}"
        RUSTFLAGS=$RF cargo doc "${ADDITIONAL_ARGS[@]}"

        mark_dry_run_published "$PACKAGE_NAME"
    else
        RUSTFLAGS=$RF cargo publish --token $TOKEN "${ADDITIONAL_ARGS[@]}"
    fi

    popd
}


do_publish_for ostd/libs/linux-bzimage/boot-params
do_publish_for ostd/libs/linux-bzimage/builder
do_publish_for ostd/libs/linux-bzimage/setup \
    --target ../builder/src/x86_64-i386_pm-none.json \
    -Zbuild-std=core,alloc,compiler_builtins \
    -Zjson-target-spec
do_publish_for osdk

TARGETS="x86_64-unknown-none"
for TARGET in $TARGETS; do
    do_publish_for ostd/libs/ostd-macros --target $TARGET
    do_publish_for ostd/libs/ostd-test --target $TARGET
    do_publish_for ostd --target $TARGET
    do_publish_for osdk/deps/frame-allocator --target $TARGET
    do_publish_for osdk/deps/heap-allocator --target $TARGET
    do_publish_for osdk/deps/test-kernel --target $TARGET

    if [ -z "$DRY_RUN" ]; then
        break
    fi
done
