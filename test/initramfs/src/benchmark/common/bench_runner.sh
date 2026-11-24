#!/bin/sh


set -e

BENCHMARK_ROOT="/benchmark"
READY_MESSAGE="The VM is ready for the benchmark."

BENCHMARK_NAME=$1
SYSTEM="${2:-asterinas}"
echo "Running benchmark: ${BENCHMARK_NAME} on ${SYSTEM}"

print_help() {
    echo "Usage: $0 <benchmark_name> <system_type>"
    echo "  benchmark_name: The name of the benchmark to run."
    echo "  system_type: The type of system to run the benchmark on. 'linux' or 'asterinas'."
}

check_benchmark_name() {
    if [ -z "${BENCHMARK_NAME}" ] || [ -z "${SYSTEM}" ]; then
        echo "Error: Invalid arguments."
        print_help
        exit 1
    fi

    local full_path="${BENCHMARK_ROOT}/${BENCHMARK_NAME}"

    if ! [ -d "${full_path}" ]; then
        echo "Directory '${BENCHMARK_NAME}' does not exist in the benchmark directory."
        print_help
        exit 1
    fi
}

prepare_system() {
    if [ ! -d /tmp ]; then
        mkdir /tmp
    fi

    if [ "$SYSTEM" = "linux" ]; then
        mount -t devtmpfs devtmpfs /dev
        mount -t proc proc /proc
        ip link set lo up
        ip link set eth0 up
        ifconfig eth0 10.0.2.15
        mount -t ext2 /dev/vda /ext2
    elif [ "$SYSTEM" = "asterinas" ]; then
        :
    else
        echo "Error: Unknown system type. Please set SYSTEM to 'linux' or 'asterinas'."
        exit 1
    fi
}

main() {
    check_benchmark_name

    prepare_system

    echo "${READY_MESSAGE}"

    BENCHMARK_SCRIPT=${BENCHMARK_ROOT}/${BENCHMARK_NAME}/run.sh
    chmod +x ${BENCHMARK_SCRIPT}
    ${BENCHMARK_SCRIPT}

    if [ "$SYSTEM" = "linux" ]; then
        poweroff -f
    fi
}

main "$@"
