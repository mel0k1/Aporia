#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Running apache bench connected to $GUEST_SERVER_IP_ADDRESS"
redis-benchmark -h $GUEST_SERVER_IP_ADDRESS -n 100000 -c 20 -t ping_inline

