#!/bin/sh


set -e

SYSFS_DIR="/sys/devices/virtual/misc/tdx_guest/measurements"

check_readable() {
    name="$1"
    path="$SYSFS_DIR/$name"
    size=$(wc -c < "$path")
    if [ "$size" -ne 48 ]; then
        echo "FAIL: $name returned $size bytes, expected 48" >&2
        exit 1
    fi
    echo "PASS: $name is readable (48 bytes)"
}

check_readable_nonzero() {
    name="$1"
    path="$SYSFS_DIR/$name"
    nz=$(od -A n -t x1 < "$path" | tr -d ' \n')
    if [ -z "$nz" ]; then
        echo "FAIL: $name is empty" >&2
        exit 1
    fi
    cleaned=$(echo "$nz" | tr -d '0')
    if [ -z "$cleaned" ]; then
        echo "FAIL: $name read back as all-zeros" >&2
        exit 1
    fi
    echo "PASS: $name is readable and non-zero"
}

check_write_rejected() {
    name="$1"
    path="$SYSFS_DIR/$name"
    if dd if=/dev/urandom bs=48 count=1 > "$path" 2>/dev/null; then
        echo "FAIL: write to read-only register $name was not rejected" >&2
        exit 1
    fi
    echo "PASS: write to read-only register $name was correctly rejected"
}

check_wrong_size_rejected() {
    name="$1"
    size="$2"
    path="$SYSFS_DIR/$name"
    if dd if=/dev/urandom bs="$size" count=1 > "$path" 2>/dev/null; then
        echo "FAIL: write of $size bytes to $name was not rejected" >&2
        exit 1
    fi
    echo "PASS: write of $size bytes to $name was correctly rejected"
}


echo "=== Read-only register smoke tests ==="
for name in mrconfigid mrowner mrownerconfig; do
    check_readable "$name"
done
check_readable_nonzero "mrtd:sha384"


echo "=== RTMR extend before/after tests ==="
i=0
while [ $i -le 3 ]; do
    name="rtmr${i}:sha384"
    path="$SYSFS_DIR/$name"
    echo "Testing RTMR${i}..."

    before=$(hd "$path")
    dd if=/dev/urandom bs=48 count=1 > "$path" 2>/dev/null
    after=$(hd "$path")

    if [ "$before" = "$after" ]; then
        echo "FAIL: RTMR${i} value did not change after extend" >&2
        exit 1
    fi
    echo "PASS: RTMR${i} value changed after extend"
    i=$((i + 1))
done


echo "=== Error path tests ==="

for name in mrconfigid mrowner mrownerconfig "mrtd:sha384"; do
    check_write_rejected "$name"
done

for size in 32 64; do
    i=0
    while [ $i -le 3 ]; do
        check_wrong_size_rejected "rtmr${i}:sha384" "$size"
        i=$((i + 1))
    done
done

echo "=== All TSM-MR tests completed successfully ==="
