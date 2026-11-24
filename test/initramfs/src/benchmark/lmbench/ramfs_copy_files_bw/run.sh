#!/bin/sh


set -e

echo "*** Running the LMbench lmdd test ***"

dd if=/dev/zero of=/tmp/zero_file bs=1M count=512
echo -n "lmdd result: " & /benchmark/bin/lmbench/lmdd if=/tmp/zero_file of=/tmp/test_file