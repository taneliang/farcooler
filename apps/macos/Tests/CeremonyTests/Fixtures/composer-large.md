`composer-large.heic` is a synthetic, opaque 6000 × 4000 image generated
locally with Core Graphics and ImageIO on macOS 27.0.1. It contains no photo,
user data, or third-party material. RGB channels at (x, y) are
`(x / 24) % 256`, `(y / 16) % 256`, and `((x + y) / 32) % 256`, with integer
division, in a Device RGB 8-bit `noneSkipLast` bitmap. ImageIO writes one HEIC
image at lossy compression quality 0.8.

The composer test reads these bytes to cover real HEIC decoding, opaque JPEG
conversion, and downscaling to 2576 pixels without needing the test runner to
have a working HEIC encoder. CI's encoder advertised HEIC but failed to
finalize a 6000 × 4000 image, returning zero bytes (ov-877).
