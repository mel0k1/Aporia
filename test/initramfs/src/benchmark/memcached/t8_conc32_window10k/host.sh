#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Running memaslap bench connected to $GUEST_SERVER_IP_ADDRESS"
memaslap -s $GUEST_SERVER_IP_ADDRESS:11211 -t 30s -T 8 -c 32 -w 10k -S 1s

