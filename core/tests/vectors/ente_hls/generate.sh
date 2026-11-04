#!/bin/bash
# Generates the Ente-format HLS preview fixture used by core's video tests.
#
# The second ffmpeg call is Ente's (mobile/apps/photos/lib/services/
# video_preview_service.dart, `_createPreview`: a random 16-byte AES key in a
# `-hls_key_info_file` whose URI line is `data:text/plain;base64,<key>`, then
# `-f hls -hls_flags single_file -hls_list_size 0`), plus `-g 15 -hls_time 1`
# so the 4 s clip has several segments. Generated with ffmpeg 6.1.1.
# `openssl_segments.sh .` writes segments.txt: every segment decrypted with
# OpenSSL (an oracle independent of core's AES code) and its SHA-256.
set -euo pipefail
out="$1"
mkdir -p "$out"
cd "$out"
ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc=size=160x120:rate=15 \
  -f lavfi -i sine=frequency=440:sample_rate=22050 -t 4 \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest src.mp4
head -c 16 /dev/urandom > keyfile.key
printf 'data:text/plain;base64,%s\n%s\n' "$(base64 -w0 keyfile.key)" "$PWD/keyfile.key" > mykey.keyinfo
ffmpeg -hide_banner -loglevel error -y -i src.mp4 \
  -vf "scale='if(lt(iw,ih),min(720,iw),-2)':'if(lt(iw,ih),-2,min(720,ih))',format=yuv420p" \
  -c:v libx264 -g 15 -maxrate 2000k -bufsize 4000k -c:a aac -b:a 128k \
  -f hls -hls_time 1 -hls_flags single_file -hls_list_size 0 \
  -hls_key_info_file mykey.keyinfo output.m3u8
