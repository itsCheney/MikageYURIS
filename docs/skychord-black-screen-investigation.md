# SkyChord patched-game investigation (2026-10-27)

The device log `Mikage-diagnostics-CEB6AEC8-1F19-4D60-9AFE-77EF04B85A74`
reports successful Category layout selection, the verified SkyChord runtime map,
and bootstrap at script 202 / PC 0. Core tables resolve from `update2.ypf`.
This is a post-bootstrap investigation, not a version-detection failure.

Downloaded the user's Drive split archives and reconstructed them in part-number
order, checking byte counts and successful YPF indexing:

| Archive | Bytes | Entries | SHA-256 |
| --- | ---: | ---: | --- |
| update1.ypf | 49017329 | 347 | e89c705dcd34a6b4cfc26c53a02293475836753cb848ea0d9c99fb3389e02966 |
| update2.ypf | 83817445 | 603 | 13d72296b82d02947220ae9ba66380fb2b338c77236d6e6d2cd803d8a2267846 |

## Confirmed decoder defect

`update2.ypf:cg/ev/ame03_22w.png` has a JPEG payload (FF D8), RGB,
1920x1080. The prior image dispatcher only accepted PNG and YCG payloads,
so it rejected this file even though archive resolution succeeded. JPEG RGB and
grayscale decoding is now selected by signature and expanded to opaque RGBA.
Synthetic tests cover misleading extensions, RGB colors, grayscale and truncated
data. The format/host suites pass (68 tests). All 290 visible PNG-named patch
images decode successfully with the fix; this is not a claim that the complete
base CG archive or the entire game works.

## Confirmed layer metadata defect

SkyChord sets `CG LINT=999 LINT2=<button index>` in script 9 (groups 965
and 1106). `es.BT.NO.GET` (script 10, groups 0–2) reads that index back
using `CGINFO LINT2`. Neither storing nor querying these fields was supported.
The initialized result stayed zero. `es.BT.SXY.GET` (script 4, groups 41–49)
therefore queried the nonexistent `.BT.OFF1` and returned a zero text-box size.

The text layout routine (script 138) first fills its coordinates with 99999,
then replaces them for characters that fit. With a zero-size box it left those
sentinels intact. Instrumented `CGACT TEXT` calls tried to draw the first Chinese
line around (100003, 99999) into a 1150x182 image. The font and dialogue data
were present; the glyphs were outside the image.

Layers now store both integers, apply CG assignments, and answer CGINFO queries.
They follow the existing attribute-copy behavior for COPY/SWAP; image-only
operations preserve destination metadata. Tests cover assignment, compound
assignment, hidden-layer queries, image edits, copying, and missing/deleted layers.

## Confirmed image-file query defect

Script 9 groups 341–359 use `FILEINFO EXIST=1 CGFILE=...` to select `_off`,
`_over`, `_on`, and other button images. Only the FILE path slot was handled.
The result initializer remained zero even though the images existed, so the
backlog return button tried to load `cgsys/backlog/btn_back` and became 0x0.

CGFILE now uses the image loader's extension-resolution list. FILE remains an
exact path query. A synthetic VM test covers extensionless and explicit image
paths, missing images, and unchanged FILE behavior. The real return-button
variants now load at 239x45. Non-ASCII path encoding is a separate unresolved
issue: the existing file-probe string conversion and VFS CP932 keys need a
consistent policy; this change does not claim to repair that path.

## Local validation and limits

Restored `pac/cgsys.ypf` (5,478,562 bytes) and `pac/cgsys2.ypf` (36,549 bytes)
from the user's Drive. The local resource set now includes ysbin, both patches,
both base UI archives, and bgm. A temporary headless Player probe used GBK,
Noto Sans SC, synthetic pointer/button input, and 100 ms simulation steps.
No gameplay archives, fonts, or game screenshots are added to this repository.

Release-build evidence:

- Frame 99: translation-group splash, without the dialogue window overlay.
- Frame 301: click title Start; frame 699: first Chinese narrative line visible.
- Frame 752: click BackLog; frame 902: Chinese history text and return button visible.
- Frame 933: click return; frame 1249: narrative interface and dialogue visible again.

The debug build did not reliably deliver the backlog click during the same
accelerated probe, while the release build completed the interaction. The Player
advances input state on each host step even when a script cycle exhausted its
wall-clock budget. Input behavior under sustained budget exhaustion needs a
separate regression investigation; it was not changed to force this probe to pass.

Regression command: `cargo test -p yuris-host -p yuris-vm -p yuris-format`: 136 tests passed.
This is a local CPU-renderer verification, not an iOS/Metal device test. Full
`cg.ypf`, voice, se, and movies are absent locally, so the black background in
these captures is not evidence that the device's background problem is fixed.
The complete CG archive is needed for that verification.

MOVIE/MOVIEINFO/MOVIEEND remain unsupported. Script 24 initializes @1988 to
zero at group 38 and breaks at groups 46–47 when playback reports zero; this
path reaches the narrative locally rather than hanging indefinitely on a movie.
Full gameplay and audio/video compatibility remain unverified.
