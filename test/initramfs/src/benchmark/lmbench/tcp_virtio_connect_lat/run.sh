#!/bin/sh


set -e

echo "Running lmbench TCP connect latency test over virtio-net..."

benchmark/bin/lmbench/lat_connect -s 10.0.2.15 -b 1000

sleep 200