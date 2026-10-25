# YSTB scripts (`ysbin\yst%05d.ybn`)

Code: `crates/yuris-format/src/ystb.rs`. One file per compiled script, named
by script number. The layout was read from the v555 engine's loader and
the official compiler (YSCom 0.495), and holds for Erewhon (v481/v490).

All integers are little-endian.

## Header (0x20 bytes, not encrypted)

| Offset | Field |
|---|---|
| 0x00 | `YSTB` |
| 0x04 | version |
| 0x08 | G: number of command groups |
| 0x0C | part1 length (= 4·G) |
| 0x10 | window table length (a multiple of 12) |
| 0x14 | content length |
| 0x18 | part4 length |
| 0x1C | 0 |

`0x20 + part1 + windows + content + part4` is the file length.

## Sections

```text
part1    G × u32         command groups
windows  Σ count × 12    parameter windows
content  bytes           expression bytecode pool
part4    bytes           continues the pool
```

**Command group** (one u32 of part1): byte 0 is the command, as an index into
the game's own YSCM table (see [ysbin.md](ysbin.md)); byte 1 is the number of
windows the group owns; the high u16 is a parameter (`gparam`). For GOSUB
and RETURN it packs how many local INT/FLT/STR slots the frame gets:
`int = (p & 0xff) >> 3`, `flt = (p & 7) * 4 + (p >> 14)`,
`str = (p >> 9) & 0x1f`.

The groups' windows follow each other in the window table: group i's windows
start where group i−1's end.

**Window** (12 bytes): `tag: u32`, `len: u32`, `offset: u32`. The offset is
relative to the start of content, and the bytes are taken from
`content ‖ part4` read as one pool: some text windows run past the end of
content into part4. Windows may overlap, and many scripts share one pool
between windows ("pooled" scripts), so the windows are not a partition of
content.

**Tag bytes** (little-endian B0..B3):

| Byte | Meaning |
|---|---|
| B0 | parameter slot, in the game's YSCM parameter order for the command |
| B1 | always 0 |
| B2 | value type the engine evaluates the window as: 0 none/raw, 1 INT (`@`), 2 FLT, 3 STR (`$`), 4 other |
| B3 | per-command extra: the compound assignment code of a LET target (0 `=`, 1 `+=`, 2 `-=`, 3 `*=`, 4 `/=`, 5 `%=`, 6 `&=`, 7 `|=`, 8 `^=`), part of an inlined label id for GOSUB |

A tag of 0 marks a raw window: an IF/ELSE branch target (its **len** field
holds a group number filled in by the compiler, not a length), a GOSUB
label name, or WORD text.

The content of a window is expression bytecode, see
[../engine/bytecode.md](../engine/bytecode.md).

## Encryption

Everything after the header is XORed with a 4-byte key, cycling, with the
index counted from the start of each section. Since part1's length is a
multiple of 4, counting from the start of the file gives the same result.

The compiler derives the key as the big-endian bytes of CRC32 of a seed
string. The SDK's default seed `Yu-ris` gives CRC32 0xd36fac96, i.e. key
bytes `d3 6f ac 96`. Games change the seed, so the key differs per game but
is the same for every script of a game:

| Game | Key |
|---|---|
| v555 sample (kemonomichi2) | `2b 90 4f 93` |
| Erewhon (Japanese scripts, v481) | `64 24 e8 dd` |
| Erewhon CHS patch (v490) | `0b 8f 00 b1` |

`guess_key` recovers it from the data: the first window's offset is 0, so
the 4 encrypted bytes at window table + 8 are the key; if the first window's
tag is 0, the bytes at window table + 0 are as well. Each candidate is
scored by how many windows decode to well-formed bytecode and by how
closely the offsets follow each other; the best is kept, and a score under
the threshold is an error rather than a guess.

## Other notes

- A script with no groups has an empty window table (12 such files in the
  v555 sample).
- In three v555 scripts the last window starts at the end of the pool with a
  non-zero length; the reader tolerates it.
