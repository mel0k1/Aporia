#!/bin/sh


set -e

./in_c/hello
./in_c_pie/hello
case "$(uname -m)" in
    x86_64|riscv64|aarch64)
        ./in_assembly/hello
        ;;
esac
