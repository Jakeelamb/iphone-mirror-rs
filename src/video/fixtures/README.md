`motion-64x96.hevc` is synthetic FFmpeg `testsrc2`, six 8-bit 64x96 frames.
It contains access-unit delimiters, repeated headers, two IDRs and predicted
pictures, with no B frames. Tests feed each complete access unit directly and
assert that a frame is returned before the next packet is supplied.

Both fixtures were generated for this project from FFmpeg's synthetic pattern;
they contain no phone recordings, personal content, or third-party footage.
They are included under the repository's GPL-3.0-or-later license. The commands
below reproduce the content and codec configuration; encoder versions may
produce different binary output.

Generated with FFmpeg 9.0.1 and libx265:

```sh
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=64x96:rate=30 \
  -frames:v 6 -pix_fmt yuv420p -c:v libx265 -preset ultrafast \
  -tune zerolatency \
  -x265-params 'pools=1:frame-threads=1:bframes=0:keyint=3:min-keyint=3:scenecut=0:repeat-headers=1:aud=1:log-level=error' \
  -f hevc motion-64x96.hevc
```

`motion-256x384.hevc` uses the same command with `size=256x384` and its matching
filename. The larger fixture meets VAAPI's minimum HEVC dimensions for the
opt-in hardware qualification test. That test compares each GPU-decoded luma
plane to the software decoder output and prints whether hardware was selected.
