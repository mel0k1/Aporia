#!/bin/bash


set -euo pipefail


SCHEME=${1:-"boot"}

KERNEL_PATH=${KERNEL_PATH:-"./target/osdk/asterinas/asterinas-osdk-bin.elf"}
INITRD_PATH=${INITRD_PATH:-"./test/initramfs/build/initramfs.cpio.gz"}
BOOT_ARGS=${BOOT_ARGS:-"console=ttyS0 earlycon loglevel=error i8042.exist"}
VCPU=${VCPU:-2}
MEM=${MEM:-1024}
FC_CONFIG_FILE=${FC_CONFIG_FILE:-/tmp/fc-vm.json}

if [ "$SCHEME" = "boot" ]; then
    BOOT_ARGS="$BOOT_ARGS"
elif [ "$SCHEME" = "ci-boot" ]; then
    BOOT_ARGS="$BOOT_ARGS rdinit=/bin/echo Entered userspace"
fi

cat > "$FC_CONFIG_FILE" << EOF
{
  "boot-source": {
    "kernel_image_path": "$KERNEL_PATH",
    "initrd_path": "$INITRD_PATH",
    "boot_args": "$BOOT_ARGS"
  },
  "drives": [],
  "machine-config": {
    "vcpu_count": $VCPU,
    "mem_size_mib": $MEM
  }
}
EOF

FC_ARGS="--no-api --config-file $FC_CONFIG_FILE"

echo $FC_ARGS
