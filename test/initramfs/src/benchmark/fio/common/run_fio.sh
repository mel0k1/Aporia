#!/bin/sh


set -e

echo FIO_RESULT_BEGIN
/benchmark/bin/fio "$@" --output-format=json --eta=never
echo FIO_RESULT_END
