# CoreDevice offer fixture

`video-offer.bin` is a 273-byte protobuf generated from the public
`pymobiledevice3` 11.13.1 `build_media_blob_video` function. It is not a device
capture, pairing record, or property list. The fixture uses `0xffffffff` as a
synthetic SSRC and the builder's default codec settings (LTRP disabled, FEC
enabled). Its other fields are static upstream protocol constants, codec
capabilities, feature strings, and the upstream default timestamp.

To regenerate in an environment with `pymobiledevice3==11.13.1` installed:

```python
from pathlib import Path
from pymobiledevice3.remote.core_device.media_stream_offer import build_media_blob_video

Path("src/device/fixtures/video-offer.bin").write_bytes(
    build_media_blob_video(0xffffffff)
)
```

SHA-256: `b7c28cd5297d5ffa344124a7f0a4d9c43ee8c57ddf110413f6c0508c05cc5b86`.
The Rust offer unit test checks exact parity with this fixture. Regeneration
from the pinned Python package also reproduced it byte for byte during the
public-release audit.

The source builder is
[`media_stream_offer.py` in pymobiledevice3 11.13.1](https://github.com/doronz88/pymobiledevice3/blob/v11.13.1/pymobiledevice3/remote/core_device/media_stream_offer.py),
licensed GPL-3.0-or-later. See the repository's
[third-party notices](../../../THIRD_PARTY_NOTICES.md).
