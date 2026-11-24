#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Running lmbench tcp client connected to $GUEST_SERVER_IP_ADDRESS"
lat_tcp -P 1 $GUEST_SERVER_IP_ADDRESS

