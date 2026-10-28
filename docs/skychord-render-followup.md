# SkyChord rendering follow-up (2026-10-28)

Source: device log `Mikage-diagnostics-9027F4E8-FF8A-4162-A620-FA73C62B15AE`.
The session boots successfully and ends about 51 seconds later with a wgpu
validation panic: `In a CommandEncoder / In a pass parameter / Encoder is invalid`.
That final message does not identify the original invalid command.

## Confirmed vertex-buffer growth defect

The renderer encoded scene draws before calling `upload_vertices`. When the
frame exceeded the current vertex-buffer capacity, upload allocated a new
buffer, but the already encoded draws still referenced the old, smaller buffer.
The initial 64 KiB buffer holds 1260 vertices (210 quads).

A synthetic regression grows a scene from 1 to 300 to 1200 layers. Running it
against the prior implementation with software Vulkan (Mesa lavapipe) fails:

```
In a draw command, kind: Draw
Vertex 1266 extends beyond limit 1260 imposed by the buffer in slot 0.
```

The renderer now prepares every draw, appends the presentation quad when needed,
allocates/uploads the final vertex buffer, and only then encodes render passes.
Both readback and presentation use this order. The growth regression covers both
entry points, and the existing GPU/CPU pixel comparison remains enabled.
Tests actually run with lavapipe; a run with no adapter is a skip, not evidence
of GPU correctness. This confirms and fixes a validation-error trigger, but an
iOS/Metal retest is still needed to attribute the device's generic final error.

## Confirmed narration/name mixing

The user's screenshot contains `写着“WARNING”的都市禁区。【梯子】` in the name
field. Although `【梯子】` itself occurs literally in the patched script, the
preceding narration must not be part of that name. The screenshot establishes
that the earlier source-literal check alone was insufficient.

SkyChord's YSCM TEXT interface contains exactly FILE, LET, CLEAR; it has no
OUTPUTMODE selector. The VM nevertheless defaulted to automatic text output,
so consecutive WORD/RETURNCODE lines were accumulated without yielding to the
script's text consumer. Script 137's name checker searched that combined string
and could extract the preceding narration together with the next speaker name.
It also allowed scenario commands to run ahead of text consumption.

The command-map initialization now selects script-managed text for this exact
compact interface. The existing line-boundary yield and end-of-text restart
protocol are reused. Identity/unknown maps and the older interface that exposes
OUTPUTMODE retain their previous default. Tests distinguish these table shapes.

A release Player probe advances through script 231 with GBK and Noto Sans SC.
With the fix, the first narrative line is isolated; the agency dialogue alternates
between `【房屋中介大叔】` and `【梯子】`. The probe continues beyond the WARNING
narration and asserts that WARNING never enters the name buffer. Local run reaches
frame 2184 without that assertion failing. Repeating the same probe with only
the compact-text mode selection disabled reproduces the screenshot's mixed
WARNING/name string and fails the assertion. This validates text state, not the
missing base CG graphics.

## Property-agency sequence and resource paths

The agency sequence in script 231 calls backgrounds `fudousan_01`, `fudousan_04`,
`fudousan_03`, `fudousan_02`, with `ブラインド・右→左` and `ブラインド・左→右`
(blind transitions). These match the failed/misdecoded transition paths in the
latest log. The earlier `スポット` transition has the same encoding issue.

Host image lookup decoded every script filename using the dialogue codepage
(GBK here), then passed the resulting UTF-8 string to a VFS keyed by original
archive filename bytes. Japanese CP932 names could not resolve. Image lookup
now preserves the original filename bytes through normalization, extension
resolution, and archive/loose-file lookup. A synthetic Japanese blind-transition
filename test verifies resolution without any game resource upload. Existing
string-based image APIs are unchanged; audio/FILEINFO path encoding is not
claimed fixed by this image-specific change.

The log also reports `CGACT SIZE: FX/FY/FSX/FSY ignored` and unsupported movie
commands. Those gaps remain. Missing local agency backgrounds and transition
images prevent visual confirmation of the complete sequence. Scene pacing may
also improve with text serialization, but the user must retest the brief black
intervals on the actual resource set. No whole-cg.ypf upload is required; if needed,
only the four named agency images and two blind-transition images can be extracted.

Validation: 140 format, host, VM, and renderer tests pass; GPU tests run on
Mesa lavapipe. Temporary game-specific probes and downloaded resources remain
outside the repository. These changes have not been validated on iOS/Metal.
