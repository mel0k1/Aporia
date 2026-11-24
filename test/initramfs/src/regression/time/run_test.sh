#!/bin/sh


set -e

./clock_nanosleep/nanosleep_err

./gettimeofday/gettimeofday

./itimer/setitimer
./itimer/timer_create

./timerfd/timerfd_clockid
./timerfd/timerfd_readiness
