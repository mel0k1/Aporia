#!/bin/bash



IP=10.0.2.2/24

if [ -n "$1" ]; then
    ip addr add $IP dev "$1"
    ip link set dev "$1" up
    exit
else
    echo "Error: no interface specified"
    exit 1
fi