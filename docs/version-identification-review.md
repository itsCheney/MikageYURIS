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
