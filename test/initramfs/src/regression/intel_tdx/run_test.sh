#!/bin/sh


set -e

if [ -e /dev/tdx_guest ]; then
    ./tsm_mr/tsm_mr.sh
fi
