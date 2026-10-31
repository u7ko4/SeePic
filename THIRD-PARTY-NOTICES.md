# Third-party notices

SeePic itself is MIT licensed (see `LICENSE`). It is built from the components
below, which keep their own licences. Nothing here changes the licence of the
SeePic source.

## Direct dependencies

| Component | Licence | Used for |
|---|---|---|
| `gpui-kit`, `gpui-pre`, `gpui-base` | Apache-2.0 | UI framework, components, themes |
| `image` | MIT OR Apache-2.0 | image decoding and encoding |
| `jpeg-decoder` | MIT OR Apache-2.0 | JPEG decoding at a reduced scale |
| `fast_image_resize` | MIT OR Apache-2.0 | SIMD resampling |
| `kamadak-exif` | BSD-2-Clause | EXIF metadata |
| `zip` | MIT | ZIP / CBZ archives |
| `sevenz-rust2` | Apache-2.0 | 7Z / CB7 archives |
| `tar` | MIT OR Apache-2.0 | TAR / CBT archives |
| `unrar` / `unrar_sys` | see below | RAR / CBR archives |
| `trash` | MIT | moving files to the recycle bin |
| `rfd` | MIT | native file dialogs |
| `raw-window-handle` | MIT OR Apache-2.0 OR Zlib | window handle access |
| `windows-sys` | MIT OR Apache-2.0 | Win32 APIs |
| `serde`, `serde_json`, `anyhow`, `thiserror` | MIT OR Apache-2.0 | configuration and errors |

The full dependency graph is recorded in `Cargo.lock`. Of the 835 locked
packages, all are permissively licensed; a few transitive crates are
`MPL-2.0` (file-level copyleft: `cbindgen`, `dwrote`, `option-ext`) or are
dual-licensed with a permissive option (`Unlicense OR MIT`,
`Apache-2.0 OR GPL-2.0-only`, `MIT OR Apache-2.0 OR LGPL-2.1-or-later`), where
the permissive branch is the one used.

## UnRAR

`unrar_sys` compiles the UnRAR C++ sources it vendors, and those sources carry
their own licence, which the crate's own `MIT OR Apache-2.0` metadata does not
cover. The licence requires the following text to be reproduced, so it is
reproduced here in full.

> UnRAR source code may be used in any software to handle RAR archives without
> limitations free of charge, but cannot be used to develop RAR (WinRAR)
> compatible archiver and to re-create RAR compression algorithm, which is
> proprietary. Distribution of modified UnRAR source code in separate form or as
> a part of other software is permitted, provided that full text of this
> paragraph, starting from "UnRAR source code" words, is included in license, or
> in documentation if license is not available, and in source code comments of
> resulting package.

The remaining terms of that licence:

> 1. All copyrights to RAR and the utility UnRAR are exclusively owned by the
>    author - Alexander Roshal.
>
> 3. The UnRAR utility may be freely distributed. It is allowed to distribute
>    UnRAR inside of other software packages.
>
> 4. THE RAR ARCHIVER AND THE UnRAR UTILITY ARE DISTRIBUTED "AS IS". NO WARRANTY
>    OF ANY KIND IS EXPRESSED OR IMPLIED. YOU USE AT YOUR OWN RISK. THE AUTHOR
>    WILL NOT BE LIABLE FOR DATA LOSS, DAMAGES, LOSS OF PROFITS OR ANY OTHER KIND
>    OF LOSS WHILE USING OR MISUSING THIS SOFTWARE.

SeePic only reads RAR archives; it does not create them and does not implement
the compression algorithm.

**This is the only component in the build that is not under a permissive
licence.** Removing RAR support (the `unrar` dependency and
`crates/seepic-core/src/archive/rar_backend.rs`) would leave the whole build
under MIT/Apache-2.0/BSD terms and this notice would no longer be needed.

## Format icons (placeholder)

`assets/formats/*.ico` — the per-format icons shown on the settings page and
written beside the registered ProgIDs — are placeholder artwork supplied for
development. They are **not** covered by SeePic's MIT licence. Confirm their
redistribution rights or replace them with original (or permissively licensed)
artwork before publishing a release.
