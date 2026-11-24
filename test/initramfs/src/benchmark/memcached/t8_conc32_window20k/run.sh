#!/bin/sh


set -e

echo "Running Memcached server"
/benchmark/bin/memcached --user=root --listen=10.0.2.15
