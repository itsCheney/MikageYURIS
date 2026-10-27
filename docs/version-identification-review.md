# YU-RIS version identification review

Research date: 2026-10-27. No runtime changes in this review.

## Evidence

- shimamura-sakura/yuri, commit 183b85e: yuri.md (Commercial versions;
  YSVR; YSCD), yuri/fileformat/yscd.py, ysvr.py, ystl.py, YSCom/*.ycd.
- ZQF-ReVN/RxYuris, commit f3d87c0: lib/YurisStaticLibrary/YSTB.cpp,
  YSTL.cpp, YSCOM_Struct.h. Useful format families, not a runtime system map.
- arcusmaximus/VNTranslationTools, commit 36fbe90:
  VNTextPatch.Shared/Scripts/Yuris/YurisCommandList.cs reads command AND
  parameter IDs by name from the game's YSCM.

The yuri documentation explicitly reports commercial header versions 554
(Unionism Quartet) and 490 (Fraternite) with pre-481 public layouts.
Its parsers accept a layout-version override independently from the header.

Parsed all 29 available YSCD dictionaries. Seven distinct complete ordered
(name,type,dimensions) tables were observed. File name and embedded header
must be retained separately: 484/487/488/493.ycd all report header 481.

| Dictionary file | _INP | _MOUSE_L | _TEXT | _CHR | _SAVE_PATH |
|---|---:|---:|---:|---:|---:|
| 450 | 92 | 95 | 70 | 75 | 57 |
| 470 | 112 | 115 | 90 | 95 | 73 |
| 480 | 134 | 137 | 112 | 117 | 95 |
| 484 (header 481) | 137 | 140 | 115 | 120 | 98 |
| 494 | 144 | 147 | 122 | 127 | 105 |

Current Mikage input constants match these selected 494 dictionary entries,
not the public 480 dictionary described in input.rs comments. This does not
invalidate commercial Erewhon observations; commercial versions can differ.
Crucially @115 is _MOUSE_L in 470, $_TASK in 480, $_TEXT in 484, and
_SCREEN_SIZE_X in 494. The reader must select semantics by mapped name and
space, not unconditionally treat numeric IDs as a fixed system interface.

## Proposed implementation sequence

1. Introduce a detection result containing reported versions for each resolved
   file (EXE optional, YSCF/YSVR/YSCM/YSTB), separately selected structural
   layouts, system-variable profile, evidence, and confidence. Resolve files
   through the existing patch-priority VFS first. Do not turn mixed reported
   versions into an automatic error.
2. Add structural candidates for YSVR: pre-481 entries have no category byte;
   newer entries have it. Validate bounds, types, byte consumption and variable
   IDs. Apply the same principle to other versioned formats. Never silently
   select the first candidate if multiple candidates remain plausible.
3. Match YSVR system entries against generated dictionary shape profiles
   (ID, prefix/type, rank, dimensions). Use complete shapes, not only INP[1024].
   YSVR has no names: shape agreement is evidence, not proof of identical
   semantics. Keep known commercial fingerprints and engine disassembly as
   stronger evidence; provide a logged explicit override for ambiguous builds.
4. Resolve named semantic variables through a SystemVarMap. Pass it to the
   shared reader, input updater, path initialization, text/character handling,
   clock and frame argument/return handling. Gate writes as well as reads.
   Remove the unknown-profile mouse fallback that writes @147 unconditionally.
5. Preserve per-archive detection of YPF layout/name key/table, per-game script
   key inference, and YSCM name-based command mapping. These are independent
   axes and must not be forced by a single engine_version field.
6. Add diagnostics containing evidence, selected profile, rejected candidates,
   conflicts, unknown semantic accesses with script/PC, and explicit overrides.

Suggested first implementation: detection-only diagnostics and profile fixture
checks, then route current known-game mappings through SystemVarMap without
claiming support for all older runtime formats. Add old-format readers after
real/synthetic tests for each layout. Do not ship the SDK binaries or game data
as fixtures; retain source/version provenance for generated factual mappings.

Tests: public 480 vs 494 shapes; misleading commercial version header;
old/new YSVR layouts; ambiguous scalar-only matches; patch-overridden tables;
unknown profile must not overwrite @147 or reinterpret @115 as display width;
current SkyChord 555 behavior preserved.

## Stage 1 implementation: read-only detection

`yuris_format::detection::DetectionReport` is collected at the start of VM boot
and logged through the existing `yuris_version` tracing target. `Booted` retains
the report. It records resolved YSCF, YSVR, YSCM, YSLB and numbered YSTB headers,
with their actual VFS origins. Mixed header versions are diagnostic evidence,
not boot failures. Script headers are summarized by version in logs; individual
read/header failures are also logged. The report retains every inspected file.

The current YSVR reader must consume the whole file before shape comparisons
are produced. Seven dictionary families cover the 29 inspected public SDK
files (450–494). Generated profiles contain only factual type/dimension data;
provenance is recorded in `system_profiles.rs`. Comparisons retain missing,
conflicting, duplicate, and unexpected active system IDs. A unique complete
shape match still does not prove semantic compatibility: YSVR contains no
variable names. Unknown and ambiguous results remain explicit.

This stage does **not** select a new parser, change runtime variable mappings,
remove existing fallback writes, inspect EXE metadata, or support the old YSVR
layout. Those remain subsequent steps. A malformed current-layout table is
reported rather than retried under an assumed version threshold.

Validation: all seven synthetic profiles uniquely match regardless of their
reported header version; one INP anchor is insufficient; duplicate IDs conflict;
patch-selected header and source agree; missing files produce evidence. Format,
VM and player test suites pass.

### SkyChord real archive validation (2026-10-27)

Rechecked the re-uploaded `ysbin.ypf` (3,017,456 bytes; SHA-256
`f7cc95f4af75fbf5cea04125f1963d93e5a4036f553b9611c9a1bcc9b59a20e7`).
All 292 inspected files (288 numbered YSTB scripts and four core tables) report
555, with no read/header failures. The four core origins resolve to this archive.
The current YSVR parser consumes the entire table.

| Dictionary family | Matched | Missing | Conflicting | Extra active IDs |
| --- | ---: | ---: | ---: | ---: |
| 450–451 | 395 | 0 | 101 | 52 |
| 452–466 | 432 | 0 | 81 | 35 |
| 470–471 | 435 | 0 | 81 | 32 |
| 472,473,477,479 | 452 | 0 | 74 | 22 |
| 480 | 469 | 0 | 69 | 10 |
| 484,487,488,493 | 481 | 0 | 60 | 7 |
| 494 | 548 | 0 | 0 | 0 |

The 494 dictionary is the sole complete **shape** match; this does not establish
semantic equivalence of every variable or make the engine version 494. The
bootstrap integration also succeeds: entry script 202, PC 0, 1,969 applied YSVR
initial values, with the same report retained in `Booted`. Existing runtime
mapping remains unchanged. This checks archive detection and VM initialization,
not full gameplay, rendering, audio, or unprovided update archives.

## Stage 2 implementation: explicit runtime mappings

`yuris_core::SystemVarMap` now separates runtime semantics from the detection
report. `SystemVarIds` centralizes the current SkyChord IDs. The shared reader,
all VM expression evaluators (including YSVR string initializers), input codes
and state updates, paths, date/time, text and character tables, and frame
argument/return slots use the selected mapping.

Automatic bootstrap selection currently recognizes only the verified SkyChord
YSVR: 47,166 decompressed bytes, FNV-1a-64 `d472c3293659379d`, plus a unique
complete 494 shape match. This is a compatibility fingerprint, not an authenticity
check. Hashing the entire table deliberately makes modified initializers or
patches unknown until verified. Neither a header of 555 nor a matching shape
alone selects runtime semantics. The file evidence remains separate and intact.

Unrecognized games select `Unknown`: computed reads fall through to ordinary
storage; host input/path/time/text/character writes and frame-slot synthesis
are disabled. In particular, the old fallback writing mouse state to @147 is
removed. The selected map and disabled semantics are logged; skipped frame
operations include script and PC. This does not claim unknown games can play
correctly. It avoids assigning semantics to ordinary IDs from another layout.

`Bootstrap::boot_with_system_var_map` provides an explicitly logged override for
independently verified games. Low-level `GroupVm::load` and `Evaluator::new`
retain the legacy standalone SkyChord behavior for existing embedding callers;
those callers can explicitly select `Unknown`. Set the map before applying YSVR
and executing scripts. An override does not rewrite the detection evidence.
Additional commercial fingerprints and old layouts remain future work.

Validation includes unknown-map preservation of @115 and @147, an insufficient
single INP anchor, text/path/time writes, frame and character slots, YSVR initial
expression reads, and explicit bootstrap overrides. SkyChord initialization
still selects the known map, enters script 202 at PC 0, applies 1,969 initial
values, and exposes its YSCF-derived 1920×1080 display metrics. This is a bootstrap
check, not a full gameplay or rendering validation.

## Stage 3 implementation: structural YSVR layout selection

`YsvrTable::from_bytes` now evaluates both `Legacy` (seven-byte entry prefix,
no category) and `Category` (eight-byte prefix). It selects only a unique valid
candidate, independently of the reported version. `from_bytes_with_layout`
provides an explicit low-level choice with identical validation. This supersedes
the earlier stage-1 restriction to the current layout.

Both readers validate header and count, scopes 1–3, types 0–3, categories 0–3,
system-ID category consistency, dimension/initializer byte boundaries, and exact
file consumption. Zero-sized dimensions and repeated IDs remain legal: they are
not sufficient evidence to reject a physical layout, and scoped/redefined
variables can repeat IDs. The reader does not allocate arrays from dimension
values. Legacy categories follow `yuri` commit 183b85e `readV000`: SYS=0 for
IDs below 1000, DEF=1 otherwise; the missing byte is not read from the stream.

Diagnostics retain both layout candidates and their rejection reasons. Zero
valid candidates mean unsupported or malformed data. Two valid candidates mean
ambiguity, including an empty table; automatic bootstrap fails rather than using
the header version to break the tie. Shape matching is withheld on ambiguity.
An explicit runtime system-variable map override does not resolve a binary-layout
ambiguity: these are separate decisions.

Synthetic fixtures cover both layouts with reported versions 450, 480, 481, 490,
554 and 555, all initializer types, multidimensional and zero-sized arrays,
truncation at every byte boundary, trailing data, invalid fields/counts, and an
ambiguous empty table. Bootstrap coverage includes a patch-selected legacy YSVR
still reporting 555, and rejection of an ambiguous table. SkyChord's actual
YSVR uniquely selects Category and retains the stage-2 bootstrap results.
No real legacy-game sample has been validated yet. Supporting its YSVR does not
imply support for its other containers, expression format, or system semantics.

## Exported startup diagnostics

Startup now emits identification evidence and a runtime-map decision before
script-key inference or label parsing can fail. Records include:

- Resolved core file paths/origins and reported versions; script version counts.
- YSVR layout status (`selected`, `ambiguous`, `unreadable`), valid candidates,
  and each rejected candidate's validation error.
- YSVR byte length/fingerprint, every dictionary's match counts, and missing,
  conflicting and extra IDs in bounded chunks of 32.
- Detected and selected semantic maps, any explicit override, fingerprint/shape
  checks, and a reason: `ysvr_missing_or_unreadable`, `ysvr_layout_unresolved`,
  `system_shape_unrecognized`, `fingerprint_not_registered`, or
  `verified_fingerprint_and_shape`.
- `boot_success` with entry script/PC and applied initial values, or
  `boot_failed` with the stage and original error. Stages cover filesystem,
  script key, labels, entry lookup/script, YSVR, commands, VM initialization,
  declarations and variable initializers.

The mobile callback already forwards `yuris_version` and `yuris_boot` through
its `yuris*` target filter. Hosts must install it before opening the player and
request INFO (`max_level >= 3`) to retain evidence, not only warnings/errors.
This repository does not contain the consuming app's JSONL export UI, so the
host's retention and share-sheet behavior still require device validation.

`yuris boot GAME --jsonl FILE` now creates the file before VFS/boot work and
captures diagnostics even when startup fails. Diagnostic rows have
`type="diagnostic"`, `schema_version=1`, `level`, `target`, and a `fields` object;
existing VM event rows follow their existing schema. Consumers must distinguish
row types. Export I/O failures are reported. The regression test opens a
synthetic game that fails key inference and verifies that ambiguous YSVR
evidence, its actual loose-file source, the unknown-map reason, and the failure
stage all survive in valid JSONL.

Export validation: 119 tests across format, VM, player and tools passed. The
SkyChord CLI smoke run produced 1,692 individually valid JSONL rows (45 diagnostic
rows and 1,647 existing VM event rows), including Category selection,
`verified_fingerprint_and_shape`, and `boot_success` at script 202 / PC 0 with
1,969 applied initial values. This verifies export and initialization, not full
gameplay; the CLI still reports an unimplemented/recorded ERROR command later.
