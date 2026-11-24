#!/bin/bash


set -e
set -o pipefail

if ! command -v yq >/dev/null 2>&1; then
    echo >&2 "Error: missing required tool: yq"
    exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
    echo >&2 "Error: missing required tool: jq"
    exit 1
fi

BENCHMARK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
source "${BENCHMARK_ROOT}/common/prepare_host.sh"
RESULT_TEMPLATE="${BENCHMARK_ROOT}/result_template.json"
BENCH_LINUX_VIRTIOFS_WORK_DIR=""
BENCH_LINUX_VIRTIOFSD_PID=""
BENCH_LINUX_VIRTIOFS_SOCKET=""
BENCH_VIRTIOFS_CACHE="auto"
VIRTIOFS_TAG="aster-virtiofs"

extract_result() {
    local file="$1" pattern="$2" column="$3" nth="$4"
    PATTERN="$pattern" awk -v column="$column" \
        '$0 ~ ENVIRON["PATTERN"] { if (column == "NF") print $NF; else print $column }' \
        "$file" | tr -d '\r' | sed 's/[^0-9.]*//g' | sed -n "${nth}p"
}

parse_raw_results() {
    local search_pattern="$1"
    local nth_occurrence="$2"
    local result_index="$3"
    local result_file="$4"
    local parser="$5"
    local direction="$6"

    local linux_result aster_result
    case "$parser" in
        text)
            linux_result=$(extract_result "$LINUX_OUTPUT" "$search_pattern" "$result_index" "$nth_occurrence")
            aster_result=$(extract_result "$ASTER_OUTPUT" "$search_pattern" "$result_index" "$nth_occurrence")
            ;;
        fio_json)
            source "${BENCHMARK_ROOT}/fio/common/parse_result.sh"
            linux_result=$(extract_fio_result "$LINUX_OUTPUT" "$direction")
            aster_result=$(extract_fio_result "$ASTER_OUTPUT" "$direction")
            ;;
        *)
            echo "Error: Unknown result parser '$parser'" >&2
            return 1
            ;;
    esac

    if [ -z "${linux_result}" ] || [ -z "${aster_result}" ]; then
        echo "Error: Failed to parse the results from the benchmark output" >&2
        exit 1
    fi

    yq --arg linux_result "${linux_result}" --arg aster_result "${aster_result}" \
        '(.[] | select(.extra == "linux_result") | .value) |= $linux_result |
         (.[] | select(.extra == "aster_result") | .value) |= $aster_result' \
        "${RESULT_TEMPLATE}" > "${result_file}"
    echo "Results written to ${result_file}"
}

generate_template() {
    local unit="$1"
    local legend="$2"

    local linux_legend=${legend//"{system}"/"Linux"}
    local asterinas_legend=${legend//"{system}"/"Asterinas"}

    yq -n --arg linux "$linux_legend" --arg aster "$asterinas_legend" --arg unit "$unit" '[
        { "name": $linux, "unit": $unit, "value": 0, "extra": "linux_result" },
        { "name": $aster, "unit": $unit, "value": 0, "extra": "aster_result" }
    ]' > "${RESULT_TEMPLATE}"
}

extract_result_file() {
    local bench_result="$1"
    local relative_path="${bench_result#*/benchmark/}"
    local first_dir="${relative_path%%/*}"
    local filename=$(basename "$bench_result")

    if [[ "$filename" == bench_* ]]; then
        local second_part=$(dirname "$bench_result" | awk -F"/benchmark/$first_dir/" '{print $2}' | cut -d'/' -f1)
        echo "result_${first_dir}-${second_part}.json"
    else
        local result_file="result_${relative_path//\//-}"
        echo "${result_file/.yaml/.json}"
    fi
}

start_linux_virtiofsd() {
    "${BENCHMARK_ROOT}/../../../../tools/run_virtiofsd.sh" \
        --cache-mode "$BENCH_VIRTIOFS_CACHE" \
        --work-dir "$BENCH_LINUX_VIRTIOFS_WORK_DIR" \
        &
    BENCH_LINUX_VIRTIOFSD_PID=$!
    local socket_wait_tries=100
    for _ in $(seq 1 "$socket_wait_tries"); do
        [[ -S "$BENCH_LINUX_VIRTIOFS_SOCKET" ]] && break
        sleep 0.1
    done
    if [[ ! -S "$BENCH_LINUX_VIRTIOFS_SOCKET" ]]; then
        echo "Error: virtiofsd did not create its socket within 10 s" >&2
        exit 1
    fi
}

cleanup_linux_virtiofs() {
    if [[ -n "$BENCH_LINUX_VIRTIOFSD_PID" ]]; then
        kill "$BENCH_LINUX_VIRTIOFSD_PID" 2>/dev/null || true
        wait "$BENCH_LINUX_VIRTIOFSD_PID" 2>/dev/null || true
    fi
    rm -rf "$BENCH_LINUX_VIRTIOFS_WORK_DIR"
}

run_benchmark() {
    local benchmark="$1"
    local run_mode="$2"
    local runtime_configs_str="$3"

    echo "Preparing libraries..."
    prepare_libs

    local smp_val=1
    local mem_val="8G"
    local aster_scheme_cmd_part="SCHEME=iommu"
    local virtiofs="off"

    while IFS='=' read -r key value; do
         if [[ -z "$key" ]]; then continue; fi
         case "$key" in
             "smp")
                 smp_val="$value"
                 ;;
             "mem")
                 mem_val="$value"
                 ;;
             "aster_scheme")
                 if [[ "$value" == "null" ]]; then
                     aster_scheme_cmd_part=""
                 else
                     aster_scheme_cmd_part="SCHEME=${value}"
                 fi
                 ;;
             "virtiofs")
                 virtiofs="$value"
                 ;;
             *)
                 echo "Warning: Unknown runtime configuration key '$key'" >&2
                 exit 1
                 ;;
         esac
     done <<< "$runtime_configs_str"

    local asterinas_cmd_arr=(make run_kernel "BENCHMARK=${benchmark}")
    [[ -n "$aster_scheme_cmd_part" && "$platform" != "tdx" ]] && asterinas_cmd_arr+=("$aster_scheme_cmd_part")
    asterinas_cmd_arr+=(
        "SMP=${smp_val}"
        "MEM=${mem_val}"
        ENABLE_KVM=1
        RELEASE_LTO=1
        NETDEV=tap
        VHOST=on
    )
    if [[ "$platform" == "tdx" ]]; then
        asterinas_cmd_arr+=(INTEL_TDX=1)
    fi

    if [[ "$virtiofs" == "on" ]]; then
        case "$BENCH_VIRTIOFS_CACHE" in
            auto|always|never|metadata) ;;
            *)
                echo "Error: invalid VIRTIOFS_CACHE value '$BENCH_VIRTIOFS_CACHE'" >&2
                exit 1
                ;;
        esac
        BENCH_LINUX_VIRTIOFS_WORK_DIR=$(mktemp -d -p /tmp asterinas-bench-linux-virtiofs-XXXXXX)
        BENCH_LINUX_VIRTIOFS_SOCKET="${BENCH_LINUX_VIRTIOFS_WORK_DIR}/vfs.sock"
        trap cleanup_linux_virtiofs EXIT
        asterinas_cmd_arr+=(
            VIRTIOFS=on
            "VIRTIOFS_CACHE=${BENCH_VIRTIOFS_CACHE}"
        )
    fi

    local linux_cmd_arr=(
        qemu-system-x86_64
        --no-reboot
        -smp "${smp_val}"
        -m "${mem_val}"
        -machine q35,kernel-irqchip=split
        --enable-kvm
        -kernel "${LINUX_KERNEL}"
        -initrd "${BENCHMARK_ROOT}/../../build/initramfs.cpio.gz"
        -drive "if=none,format=raw,id=x0,file=${BENCHMARK_ROOT}/../../build/ext2.img"
        -device "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off"
        -drive "if=none,format=raw,id=nvme0n1,file=${BENCHMARK_ROOT}/../../build/nvme0n1.img"
        -device "nvme,drive=nvme0n1,serial=nvme0n1"
        -device "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off,csum=off,ctrl_guest_offloads=off,ctrl_mac_addr=off,ctrl_rx_extra=off,ctrl_rx=off,ctrl_vlan=off,ctrl_vq=off,event_idx=off,guest_announce=off,guest_csum=off,guest_ecn=off,guest_tso4=off,guest_tso6=off,guest_ufo=off,guest_uso4=off,guest_uso6=off,host_ecn=off,host_tso4=off,host_tso6=off,host_ufo=off,host_uso=off,indirect_desc=off,mrg_rxbuf=off,queue_reset=off"
        -append "console=ttyS0 rdinit=/benchmark/common/bench_runner.sh ${benchmark} linux mitigations=off hugepages=0 transparent_hugepage=never quiet"
        -netdev "tap,id=net01,script=${BENCHMARK_ROOT}/../../../../tools/net/qemu-ifup.sh,downscript=${BENCHMARK_ROOT}/../../../../tools/net/qemu-ifdown.sh,vhost=on"
        -nographic
    )
    if [[ "$platform" != "tdx" ]]; then
        linux_cmd_arr+=(
            -cpu Icelake-Server,-pcid,+x2apic
        )
    else
        linux_cmd_arr+=(
            -machine confidential-guest-support=tdx0
            -cpu host,-kvm-steal-time,pmu=off
            -bios "${OVMF_DIR:-/root/ovmf/release}/OVMF.fd"
            -nodefaults
            -serial stdio
            -object '{ "qom-type": "tdx-guest", "id": "tdx0", "sept-ve-disable": true, "quote-generation-socket": { "type": "vsock", "cid": "1", "port": "4050" } }'
        )
    fi

    if [[ "$virtiofs" == "on" ]]; then
        linux_cmd_arr+=(
            -object "memory-backend-memfd,id=mem0,size=${mem_val},share=on"
            -numa node,memdev=mem0
            -chardev "socket,id=char0,path=${BENCH_LINUX_VIRTIOFS_SOCKET}"
            -device "vhost-user-fs-pci,chardev=char0,tag=${VIRTIOFS_TAG}"
        )
    fi

    case "${run_mode}" in
        "guest_only")
            echo "Running benchmark ${benchmark} on Asterinas..."
            "${asterinas_cmd_arr[@]}" 2>&1 | tee "${ASTER_OUTPUT}"
            prepare_fs "$benchmark"
            if [[ "$virtiofs" == "on" ]]; then
                start_linux_virtiofsd
            fi
            echo "Running benchmark ${benchmark} on Linux..."
            "${linux_cmd_arr[@]}" 2>&1 | tee "${LINUX_OUTPUT}"
            ;;
        "host_guest")
            local asterinas_cmd_str
            printf -v asterinas_cmd_str '%q ' "${asterinas_cmd_arr[@]}"
            local linux_cmd_str
            printf -v linux_cmd_str '%q ' "${linux_cmd_arr[@]}"

            if [[ "$virtiofs" == "on" ]]; then
                start_linux_virtiofsd
            fi
            echo "Running benchmark ${benchmark} on host and guest..."
            bash "${BENCHMARK_ROOT}/common/host_guest_bench_runner.sh" \
                "${BENCHMARK_ROOT}/${benchmark}" \
                "${asterinas_cmd_str}" \
                "${linux_cmd_str}" \
                "${ASTER_OUTPUT}" \
                "${LINUX_OUTPUT}" \
                "${benchmark}"
            ;;
        *)
            echo "Error: Unknown benchmark type '${run_mode}'" >&2
            exit 1
            ;;
    esac
}

parse_results() {
    local bench_result="$1"

    local search_pattern nth_occurrence result_index unit legend parser direction
    search_pattern=$(yq -r '.result_extraction.search_pattern // empty' "$bench_result")
    nth_occurrence=$(yq -r '.result_extraction.nth_occurrence // 1' "$bench_result")
    result_index=$(yq -r '.result_extraction.result_index // empty' "$bench_result")
    unit=$(yq -r '.chart.unit // empty' "$bench_result")
    legend=$(yq -r '.chart.legend // "{system}"' "$bench_result")
    parser=$(yq -r '.result_extraction.parser // "text"' "$bench_result")
    direction=$(yq -r '.result_extraction.direction // empty' "$bench_result")

    if [[ "$parser" == "fio_json" && "$unit" != "MB/s" ]]; then
        echo "Error: The fio_json parser produces results in MB/s." >&2
        return 1
    fi

    generate_template "$unit" "$legend"
    parse_raw_results "$search_pattern" "$nth_occurrence" "$result_index" "$(extract_result_file "$bench_result")" "$parser" "$direction"
}

cleanup() {
    echo "Cleaning up..."
    rm -f "${LINUX_OUTPUT}" "${ASTER_OUTPUT}" "${RESULT_TEMPLATE}"
}

main() {
    local benchmark="$1"
    local platform="$2"

    if [[ -z "${BENCHMARK_ROOT}/${benchmark}" ]]; then
        echo "Error: No benchmark specified" >&2
        exit 1
    fi
    echo "Running benchmark $benchmark..."

    local run_mode="guest_only"
    [[ -f "${BENCHMARK_ROOT}/${benchmark}/host.sh" ]] && run_mode="host_guest"

    local bench_result="${BENCHMARK_ROOT}/${benchmark}/bench_result.yaml"
    local runtime_configs_str=""

    if [[ -f "$bench_result" ]]; then
        runtime_configs_str=$(yq -r '(.runtime_config // {}) | to_entries | .[] | .key + "=" + (.value | tostring)' "$bench_result")
    else
        for job_yaml in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            if [[ -f "$job_yaml" ]]; then
                echo "Reading runtime configurations from $job_yaml..."
                runtime_configs_str=$(yq -r '(.runtime_config // {}) | to_entries | .[] | .key + "=" + (.value | tostring)' "$job_yaml")
                if [[ -n "$runtime_configs_str" ]]; then
                    break
                fi
            fi
        done
    fi

    run_benchmark "$benchmark" "$run_mode" "$runtime_configs_str"

    if [[ -f "$bench_result" ]]; then
        parse_results "$bench_result"
    else
        for job in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            [[ -f "$job" ]] && parse_results "$job"
        done
    fi

    cleanup
    echo "Benchmark completed successfully."
}

main "$@"
