#!/bin/sh


set -e

echo "*** Running lmbench-syscall-write ***"

/benchmark/bin/lmbench/lat_syscall -P 1 write