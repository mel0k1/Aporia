#!/bin/sh


set -e

echo "*** Running boot_lat_500MB benchmark ***"
UPTIME=$(awk '{print $1}' /proc/uptime)
echo "Boot time: ${UPTIME} seconds"
