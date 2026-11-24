#!/bin/sh


set -e

echo "*** Running lmbench-getpid ***"

/benchmark/bin/lmbench/lat_syscall -P 1 null