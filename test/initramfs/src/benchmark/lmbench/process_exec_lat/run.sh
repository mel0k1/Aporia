#!/bin/sh


set -e

echo "*** Running the LMbench exec latency test ***"

cp /benchmark/bin/lmbench/hello /tmp/
/benchmark/bin/lmbench/lat_proc -P 1 exec