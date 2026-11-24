#!/bin/bash


set -e

BENCHMARK_PATH=$1
ASTERINAS_GUEST_CMD=$2
LINUX_GUEST_CMD=$3
ASTERINAS_OUTPUT=$4
LINUX_OUTPUT=$5
BENCHMARK_NAME=$6
READY_MESSAGE="The VM is ready for the benchmark."

BENCHMARK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)/../"
source "${BENCHMARK_ROOT}/common/prepare_host.sh"

if [[ "$BENCHMARK_PATH" =~ "iperf" ]]; then
    export IPERF_PORT=5201
elif [[ "$BENCHMARK_PATH" =~ "nginx" ]]; then
    export NGINX_PORT=8080
elif [[ "$BENCHMARK_PATH" =~ "redis" ]]; then
    export REDIS_PORT=6379
elif [[ "$BENCHMARK_PATH" =~ "tcp_virtio_lat" ]]; then
    export LMBENCH_TCP_LAT_PORT=31234
elif [[ "$BENCHMARK_PATH" =~ "tcp_virtio_bw" ]]; then
    export LMBENCH_TCP_BW_PORT=31236
elif [[ "$BENCHMARK_PATH" =~ "memcached" ]]; then
    export MEMCACHED_PORT=11211
fi

run_benchmark() {
    local guest_cmd=$1
    local output_file=$2
    local guest_log_file=$3
    local ready_message=$4

    echo "Running the benchmark on the VM..."
    eval "${guest_cmd}" | tee "${guest_log_file}" &

    echo "Waiting for the ready message: ${ready_message}"
    while true; do
        if grep -q "${ready_message}" "${guest_log_file}"; then
            echo "Ready message detected. Running the benchmark on the host..."
            break
        fi
        sleep 1
    done

    sleep 1

    export GUEST_SERVER_IP_ADDRESS=10.0.2.15

    bash "${BENCHMARK_PATH}/host.sh" 2>&1 | tee "${output_file}"

    rm -f "${guest_log_file}"
}

run_benchmark "${ASTERINAS_GUEST_CMD}" "${ASTERINAS_OUTPUT}" "/tmp/asterinas.log" "${READY_MESSAGE}"

wait

prepare_fs "${BENCHMARK_NAME}"
run_benchmark "${LINUX_GUEST_CMD}" "${LINUX_OUTPUT}" "/tmp/linux.log" "${READY_MESSAGE}"

wait