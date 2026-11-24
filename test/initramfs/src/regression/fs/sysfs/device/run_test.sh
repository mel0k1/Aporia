#!/bin/sh


set -e

cd "$(dirname "$0")"

./mem
./pci

echo "All device model tests passed."
