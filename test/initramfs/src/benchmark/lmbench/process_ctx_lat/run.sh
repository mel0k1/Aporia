#!/bin/sh


set -e

echo "*** Running the LMbench context switch latency test ***"

/benchmark/bin/lmbench/lat_ctx -P 1 18