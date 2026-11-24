#!/bin/sh


set -e

cp /benchmark/nginx/nginx.conf /benchmark/nginx/conf/
mkdir -p /var/log/nginx

echo "Running nginx server"
/benchmark/bin/nginx -c /benchmark/nginx/conf/nginx.conf
