#!/bin/sh


set -e

echo "*** Running the LMbench semaphore latency test ***"

/benchmark/bin/lmbench/lat_sem -P 1 -N 21