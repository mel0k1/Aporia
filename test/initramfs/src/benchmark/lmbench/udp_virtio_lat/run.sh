#!/bin/sh


set -e

echo "Running lmbench UDP latency test over virtio-net..."

/benchmark/bin/lmbench/lat_udp -s 10.0.2.15

sleep 200