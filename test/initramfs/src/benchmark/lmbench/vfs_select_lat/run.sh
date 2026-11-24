#!/bin/sh


set -e

echo "*** Running the LMbench select file latency test ***"

/benchmark/bin/lmbench/lat_select -P 1 file