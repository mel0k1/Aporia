#!/bin/sh


set -e

./futex/futex_requeue

./pipe/pipe_err
./pipe/process_pipe_available
./pipe/short_rw

./sem/sem

./shm/posix_shm
