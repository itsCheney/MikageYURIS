# YPF archives

Code: `crates/yuris-format/src/ypf.rs`. The layout is the public one
(shimamura-sakura/yuri's notes and VNTranslationTools' `Yuris/Notes.txt`
describe it); it was checked against the v500 sample and Erewhon (v481, v490
and v479 archives).

All integers are little-endian.

## Header (0x20 bytes)

| Offset | Type | Field |
|---|---|---|
| 0x00 | char[4] | `YPF\0` |
| 0x04 | u32 | version (481, 500, ...) |
| 0x08 | u32 | entry count |
| 0x0C | u32 | header_size: the end of the index. From version 300 on it counts from the start of the file; before that it leaves out the 0x20-byte header |
| 0x10 | u8[16] | zero |

## Entries

The index follows the header, one entry after another:

| Type | Field |
|---|---|
| u32 | name_hash: hash of the decrypted, lowercase name. murmur2 from about 0.477, CRC32 from 0.265, Adler-32 before |
| u8 | name_size, encrypted: `NAME_SIZE_MAP[byte ^ 0xFF]` |
| u8[name_size] | name, each byte XOR name_key; CP932, `\` separated |
| u8 | file_type, follows the extension (ogg/png/ybn ...) |
| u8 | compression: 0 stored, 1 zlib |
| u32 | uncompressed size |
| u32 | stored size |
| u32 or u64 | absolute data offset; u64 from about 0.477 |
| u32 | hash of the uncompressed data (not checked) |

`NAME_SIZE_MAP` is the identity with these pairs swapped: (3,72) (17,25)
(46,50) (6,53) (9,11) (12,16) (13,19) (21,27) (28,30) (32,35) (38,41)
(44,47).

## What the reader infers

Commercial builds report versions that do not always match their layout, so
the reader works out two things from the archive itself:

- **Offset width.** It tries the width the version suggests, then the other
  one, and keeps the one whose index ends exactly at header_size (or
  header_size + 0x20) with every entry's data inside the file.
- **name_key.** It is usually 0xFF (Erewhon), but the v500 sample uses 0xC9.
  The reader tries every key on the first entries and keeps the one whose
  names hash to the stored name_hash (any of the three hashes, as stored or
  lowercased). If none matches, it votes on the key that puts a `.` before
  the extension.

## Name lookup

Names are compared after turning `/` into `\` and lowercasing ASCII. The
trail byte of a CP932 double-byte character is left alone, even when it
falls in `A`-`Z` or is `\`.

Scripts often leave out extensions (`cgsys/main/tip_meswindow`); the game file
system completes them (`GameFs::resolve_with_ext`).

## Archives inside other files

A translation patch can ship an archive inside an Enigma Virtual Box package
(see [evb.md](evb.md)). `YpfReader::open_at` reads it in place at a base
offset; the entry offsets stay relative to the archive's own start.
