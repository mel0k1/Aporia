#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Running lmbench tcp client connected to $GUEST_SERVER_IP_ADDRESS"
bw_tcp -m 65536 -P 1 -W 50000000 -N 100 $GUEST_SERVER_IP_ADDRESS

