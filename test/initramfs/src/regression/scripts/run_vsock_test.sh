#!/bin/sh



set -e

VSOCK_DIR=/test/network/vsock
cd ${VSOCK_DIR}

echo "Start vsock test......"
./vsock_client
./vsock_server
echo "Vsock test passed."
