#!/bin/sh


set -e

EPOLLTEST_DIR=/test/io/epoll
cd ${EPOLLTEST_DIR}

echo "Start epoll_pwait test......"

./epoll_pwait &
EPOLL_PID=$!

echo "epoll_pwait PID: $EPOLL_PID"

sleep 1

kill -USR1 $EPOLL_PID
echo "Sent SIGUSR1 to PID $EPOLL_PID"

sleep 3


echo "Test completed."
