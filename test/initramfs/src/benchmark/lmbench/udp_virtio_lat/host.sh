#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Warm up......"
lat_udp -P 1 -N 10 $GUEST_SERVER_IP_ADDRESS >/dev/null 2>&1
echo "Running lmbench udp client connected to $GUEST_SERVER_IP_ADDRESS"
lat_udp -P 1 -N 10 $GUEST_SERVER_IP_ADDRESS

