#!/bin/bash


set -e
set -o pipefail

BENCHMARK_ROOT="${BENCHMARK_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." &>/dev/null && pwd)}"
LINUX_OUTPUT="${BENCHMARK_ROOT}/linux_output.txt"
ASTER_OUTPUT="${BENCHMARK_ROOT}/aster_output.txt"
LINUX_DEPENDENCIES_DIR="/opt/linux_binary_cache"
LINUX_KERNEL="${LINUX_DEPENDENCIES_DIR}/vmlinuz"
LINUX_KERNEL_VERSION="6.16.0"
LINUX_MODULES_DIR="${BENCHMARK_ROOT}/../build/initramfs/lib/modules/${LINUX_KERNEL_VERSION}/kernel"
WGET_SCRIPT="${BENCHMARK_ROOT}/../../../tools/atomic_wget.sh"

prepare_libs() {
    mkdir -p "${LINUX_DEPENDENCIES_DIR}"

    declare -A files=(
        ["${LINUX_KERNEL}"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8e6e8ad382aac7e605d7a058811d09bce478cd7d/${LINUX_KERNEL_VERSION}/vmlinuz"
    )

    for file in "${!files[@]}"; do
        if [ ! -f "$file" ]; then
            echo "Downloading ${file##*/}..."
            ${WGET_SCRIPT} "$file" "${files[$file]}" || {
                echo "Failed to download ${file##*/}."
                exit 1
            }
        fi
    done
}

prepare_fs() {
    local benchmark="$1"
    if [[ -z "${benchmark}" ]]; then
        echo "Error: benchmark name is required to prepare the Linux filesystem." >&2
        return 1
    fi

    mke2fs -F -O ^ext_attr -O ^resize_inode -O ^dir_index ${BENCHMARK_ROOT}/../../build/ext2.img
    make initramfs BENCHMARK=${benchmark}
}