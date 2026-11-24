#!/bin/sh


set -e

echo "*** Running the LMbench memory-copy bandwidth test ***"

/benchmark/bin/lmbench/bw_mem -P 1 -N 50 512m fcp