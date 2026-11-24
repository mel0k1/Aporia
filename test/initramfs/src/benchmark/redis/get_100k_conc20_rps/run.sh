#!/bin/sh


set -e

echo "Running redis server"
/benchmark/bin/redis-server /etc/redis.conf
