#!/bin/sh


set -e

echo "*** Running the LMbench shell latency test ***"

cp /benchmark/bin/lmbench/hello /tmp/
/benchmark/bin/lmbench/lat_proc -P 1 shell