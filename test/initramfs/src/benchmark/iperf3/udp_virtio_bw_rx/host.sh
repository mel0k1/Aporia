#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Running iperf3 client connected to $GUEST_SERVER_IP_ADDRESS"
iperf3 -u -b 0 -l 1460 -c $GUEST_SERVER_IP_ADDRESS -f m

