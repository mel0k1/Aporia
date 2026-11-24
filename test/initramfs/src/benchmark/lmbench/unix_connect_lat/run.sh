#!/bin/sh


set -e

echo "*** Running the LMbench unix connect latency test ***"

/benchmark/bin/lmbench/lat_unix_connect -s
/benchmark/bin/lmbench/lat_unix_connect -P 1