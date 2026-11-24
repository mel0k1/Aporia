#!/bin/bash



if [ -n "$1" ]; then
    ip link set dev "$1" down
    ip link delete dev "$1"
    exit
else
    echo "Error: no interface specified"
    exit 1
fi