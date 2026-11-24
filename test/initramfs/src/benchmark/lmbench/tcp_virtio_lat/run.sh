#!/bin/sh


set -e

echo "Running lmbench TCP latency test over virtio-net..."

/benchmark/bin/lmbench/lat_tcp -s 10.0.2.15 -b 1

sleep 200
