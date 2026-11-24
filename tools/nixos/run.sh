#!/bin/sh



set -e

usage() {
    echo "Usage: $0 [nixos | iso]"
    exit 1
}

if [ "$#" -ne 1 ]; then
    usage
fi

MODE=$1
TARGET_ARCH=${TARGET_ARCH:-x86_64}
SCRIPT_DIR=$(dirname "$0")
ASTERINAS_DIR=$(realpath "${SCRIPT_DIR}/../..")

if [ "${TARGET_ARCH}" != "x86_64" ]; then
    echo "Error: tools/nixos/run.sh currently supports only TARGET_ARCH=x86_64; got ${TARGET_ARCH}" >&2
    exit 1
fi

cd "${ASTERINAS_DIR}"

QEMU_ARGS=$(${ASTERINAS_DIR}/tools/qemu_args.sh common 2>/dev/null)

case "$MODE" in
    nixos)
        NIXOS_DIR="${ASTERINAS_DIR}/target/nixos"
        QEMU_ARGS="${QEMU_ARGS} \
            -drive if=none,format=raw,id=u0,file=${NIXOS_DIR}/asterinas.img \
            -device virtio-blk-pci,drive=u0,disable-legacy=on,disable-modern=off \
        "
        ;;
    iso)
        ASTER_IMAGE_PATH=${ASTERINAS_DIR}/target/nixos/asterinas.img
        NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-16384}
        ISO_IMAGE_PATH=$(find "${ASTERINAS_DIR}/target/nixos/iso_image/iso" -name "*.iso" | head -n 1)

        if [ ! -f "$ISO_IMAGE_PATH" ]; then
            echo "Error: ISO_IMAGE not found!"
            exit 1
        fi

        rm -f "${ASTER_IMAGE_PATH}"
        echo "Creating image at ${ASTER_IMAGE_PATH} of size ${NIXOS_DISK_SIZE_IN_MB}MB......"
        truncate -s "${NIXOS_DISK_SIZE_IN_MB}M" "${ASTER_IMAGE_PATH}"
        echo "Image created successfully!"

        QEMU_ARGS="${QEMU_ARGS} \
            -cdrom ${ISO_IMAGE_PATH} -boot d \
            -drive if=none,format=raw,id=u0,file=${ASTER_IMAGE_PATH} \
            -device virtio-blk-pci,drive=u0,disable-legacy=on,disable-modern=off \
        "
        ;;
    *)
        usage
        ;;
esac

if [ "${ENABLE_KVM}" = "1" ]; then
    QEMU_ARGS="${QEMU_ARGS} -accel kvm"
fi

QEMU_BIN=${QEMU_BIN:-qemu-system-${TARGET_ARCH}}

KERNEL_SUCCESS_EXIT_CODE=16
QEMU_SUCCESS_EXIT_CODE=$(((KERNEL_SUCCESS_EXIT_CODE << 1) | 1))

${QEMU_BIN} ${QEMU_ARGS} || exit_code=$?
exit_code=${exit_code:-0}

if [ ${exit_code} -eq 0 ] || [ ${exit_code} -eq ${QEMU_SUCCESS_EXIT_CODE} ]; then
    exit 0
fi

exit ${exit_code}
