# The `ysbin` tables

Besides the scripts (`yst%05d.ybn`, see [ystb.md](ystb.md)), the compiler
writes a few tables into `ysbin\`. They are stored in `bn.ypf` with the
scripts and are not encrypted. All integers are little-endian; strings are
CP932.

| File | Magic | Code | Content |
|---|---|---|---|
| `ysc.ybn` | YSCM | `yuris-format/src/yscm.rs` | the command table |
| `ysl.ybn` | YSLB | `yuris-format/src/yslb.rs` | labels |
| `ysv.ybn` | YSVR | `yuris-format/src/ysvr.rs` | variable declarations and initial values |
| `yst_list.ybn` | YSTL | `yuris-format/src/ystl.rs` | script list |
| `yscfg.ybn` | YSCF | `yuris-format/src/yscf.rs` | project config |
| `yst.ybn` | YSTD | none | totals |
| `yse.ybn` | YSER | none | error messages |

Save data (`save\*.sd`, YSSD) is described at the end.

## YSCM: commands (`ysc.ybn`)

The command table of the script language: every command's name and its
parameters, in the order scripts index them.

```text
u8  magic[4] = "YSCM"
u32 version
u32 command_count
u32 0
command × command_count
  cstring name
  u8      param_count
  param × param_count
    cstring name          // may be empty
    u16     type
tail
```

A group's byte 0 in a script is an index into this table, and a window
tag's byte 0 is an index into that command's parameters. Both orders change
between engine versions and custom builds: the v555 sample has 121
commands starting `ALIAS, CG, CGACT, ...` with END at 0x0d, while Erewhon
(v481) has 119 starting `CDDA, CG, CGACT, ...`, an extra `EMOTEINFO`, END at
0x0e and different TEXT parameters. The VM is written against the v555
order and remaps every script by command and parameter name when it loads
it (`yuris-vm/src/commands.rs`).

**Parameter type.** The low byte is how the engine's parameter evaluator
treats the value (0 integer, 1 string, 2 kept unevaluated); the high byte
picks a range check (1 and 2 lower bounds, 3 to 0x17 entries of a
min/max table, 0x18 to 0x1c special checks such as "below the array
size"). The VM does not use the type: window tags carry the value type
themselves.

**Tail.** 37 C strings (the C runtime's error messages, Japanese) and a
256-byte table, 1045 bytes in both games. The engine reads exactly 37
strings (a do-while over `i < 0x91` in steps of 4). The table's meaning is
not known; the reader keeps both as they are.

## YSLB: labels (`ysl.ybn`)

```text
u8  magic[4] = "YSLB"
u32 version
u32 count
u32 buckets[256]          // first label index for each hash >> 24
label × count
  u8  name_len
  u8  name[name_len]      // e.g. "es.BT.W.GET"
  u32 hash                // murmur2(name), seed 0
  u32 target              // group index in the script
  u16 script              // script number
  u8  flag
```

Labels are sorted by hash; the engine finds a name by hashing it and
scanning from its bucket. The engine copies 13 + len bytes per label into a
16-byte struct, so a fourth byte after `flag` is read from the next label;
the file has 12 + len bytes per label. The v555 sample's 4153 labels and
Erewhon's all hash with murmur2. Older engines are said to use CRC32 or
Adler-32 here as in YPF; we have not seen one.

GO, GOSUB and LABELINFO look labels up here; the entry label is
`SYSTEM_START` by default.

## YSVR: variables (`ysv.ybn`)

At run time variables have no names, only ids. YSVR declares them:

```text
u8  magic[4] = "YSVR"
u32 version
u16 count
entry × count
  u8  kind                // 1 global, 2 script-local, 3 other
  u8  category            // save category
  u16 script              // owning script
  u16 var_id
  u8  type                // 0 declared only, 1 INT, 2 FLT, 3 STR
  u8  dims
  u32 bounds[dims]
  initial value           // INT: i64, FLT: f64, STR: u16 len + bytes, type 0: none
```

- Kinds 1 and 3 are applied at boot; kind 2 entries are applied the first
  time their script is loaded. (The v555 engine's applier was reversed;
  kind 3 holds arrays that other scripts read at boot.)
- A scalar is an array with bounds `[1]`: its value is element 0.
- In Erewhon (v481) a STR initial value is a compiled expression rather
  than plain text: `M len "text"` for a literal and `V ...` for an array
  read. The VM evaluates it and drops the literal's quotes; other bytes are
  taken as plain text.
- Category is what SAVE and the save filters select by (1 to 4).

Erewhon has 2872 entries with ids up to 5494; the v555 sample 3362 with ids
up to 7570.

## YSTL: script list (`yst_list.ybn`)

```text
u8  magic[4] = "YSTL"
u32 version
u32 count
entry × count
  u32 id                  // script number, = the entry's position
  u32 source_len
  u8  source[source_len]  // e.g. "data\script\eris\button\global_f.yst"
  u64 timestamp           // source file time
  i32 variables           // -1 when the source produced no script
  i32 labels
  i32 texts
```

SYSTEMINFO answers SCRIPTNUM, SCRIPTFILE and TEXTNUM from it; the text
counts size the read-tracking tables. Erewhon has 426 entries, 8 of them
with -1 counts and no `yst%05d.ybn`.

## YSCF: project config (`yscfg.ybn`)

| Offset | Field |
|---|---|
| 0x00 | `YSCF` |
| 0x04 | version |
| 0x10 | screen width |
| 0x14 | screen height |
| 0x1C | 8 image format slots |
| 0x24 | 4 sound format slots |
| 0x3C | file priority in develop, debug and release mode (3 × u32) |
| 0x4C | u16 caption length, then the caption |

The layout follows VNTranslationTools' notes. A file priority of 1 makes the
engine read loose files ahead of archives; release builds normally have 0.
The order of the first two priorities is not confirmed (the samples have
the same value in both). Erewhon's screen is 1152 × 720.

## YSTD: totals (`yst.ybn`)

```text
u8  magic[4] = "YSTD"
u32 version
u32 a                     // Erewhon 5559, v555 sample 7610
u32 texts                 // total text count
```

`texts` is the sum of YSTL's text counts (31729 in Erewhon; 0 in the v555
sample). `a` is just above the highest YSVR variable id in both games, so
it is probably the size of the variable table; nothing reads it yet.

## YSER: error messages (`yse.ybn`)

```text
u8  magic[4] = "YSER"
u32 version
u32 count
u32 0
entry × count
  u32     code            // 100010, 100030, ... 901040
  cstring message         // "%s" placeholders for file names and the like
```

Erewhon has 123 entries and the list ends exactly at the end of the file.
The runtime does not read it; our errors are our own.

## YSSD: save data (`save\*.sd`)

Code: `yuris-format/src/yssd.rs`. The engine's SAVE and LOAD commands store
variables in numbered blocks (`DNO`). From the v555 sample's save folder:

```text
u8  magic[4] = "YSSD"
u32 version               // 0x1E0
u32 block_count
u32 table_len             // 0x400
u32 offsets[0x400]        // offsets[i] = file offset of block i+1; 0 = empty
block
  u32 dno
  u8  type                // 0 single block
  u8  strict              // 1: the dimensions must match the declaration
  u16 var_id
  u32 packed_len
  u32 unpacked_len
  u8  data[packed_len]    // SNP-compressed
payload (unpacked)
  u32 type                // 1 INT, 2 FLT, 3 STR
  u32 dims
  u32 bounds[dims]
  u32 data_len
  data                    // INT/FLT: 8 bytes per element; STR: u32 len + bytes per element
```

SNP (`YSSNP.DLL`) is raw Snappy: a varint with the unpacked length, then
literal and copy elements. `snp_uncompress` decodes it; the v555 sample's
saves all use it (none are zlib).
