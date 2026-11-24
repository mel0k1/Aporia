#!/bin/sh



LEN=$1

if ! [ "$LEN" -eq "$LEN" ] || [ "$LEN" -lt 120 ]; then
    echo "Error: LEN must be a numeric value greater than or equal to 120"
    exit 1
fi

DIRNAME=/benchmark/nginx/html
mkdir -p $DIRNAME
FILENAME=${DIRNAME}/${LEN}bytes.html

rm -f ${FILENAME}

HEADER_CONTENT="<!DOCTYPE html>
<html>
<head>
<title>Sample Page</title>
</head>
<body>
<h1>Hello World!</h1><p>"

echo "$HEADER_CONTENT" > $FILENAME

HEADER_LENGTH=${#HEADER_CONTENT}
FOOTER_CONTENT="</p>
</body>
</html>"
CONTENT_LENGTH=$((LEN - HEADER_LENGTH - ${#FOOTER_CONTENT}-2))

if [ "$CONTENT_LENGTH" -gt 0 ]; then
    i=0
    while [ "$i" -lt "$CONTENT_LENGTH" ]
    do
        echo -n "a" >> $FILENAME
        i=$((i + 1))
    done
fi

echo "$FOOTER_CONTENT" >> $FILENAME
