#!/bin/sh


set -e

echo "Running lmbench TCP bandwidth test over virtio-net..."

/benchmark/bin/lmbench/bw_tcp -s 10.0.2.15 -b 1

sleep 200