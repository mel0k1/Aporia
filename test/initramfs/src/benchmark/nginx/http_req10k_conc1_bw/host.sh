#!/bin/sh


set -e

stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs -r kill
}

trap stop_guest EXIT

echo "Running apache bench connected to $GUEST_SERVER_IP_ADDRESS"
ab -n 10000 -c 1 http://$GUEST_SERVER_IP_ADDRESS:8080/index.html

