#!/bin/sh


set -e

echo "*** Running lmbench-syscall-read ***"

/benchmark/bin/lmbench/lat_syscall -P 1 read