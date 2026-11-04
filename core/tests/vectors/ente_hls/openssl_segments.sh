#!/bin/bash
# SHA-256 of each segment of the fixture, decrypted independently with OpenSSL.
set -euo pipefail
dir="$1"
key=$(grep -o 'base64,[^"]*' "$dir/output.m3u8" | cut -d, -f2 | base64 -d | od -An -tx1 | tr -d " \n")
iv=00000000000000000000000000000000
grep BYTERANGE "$dir/output.m3u8" | sed 's/.*://' | while IFS=@ read -r len off; do
  dd if="$dir/output.ts" bs=1 skip="$off" count="$len" status=none \
    | openssl enc -d -aes-128-cbc -K "$key" -iv "$iv" > "$dir/seg.plain"
  echo "$off $len $(stat -c %s "$dir/seg.plain") $(sha256sum "$dir/seg.plain" | cut -d' ' -f1) $(head -c 1 "$dir/seg.plain" | od -An -tx1 | tr -d " \n")"
done
rm -f "$dir/seg.plain"
