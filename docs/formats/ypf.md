# YPF 封包格式规格

| 项 | 值 |
|---|---|
| 状态 | **Confirmed**（公开格式；样本 v500 的首条 name_hash 与 `murmur2("ysbin\yst00034.ybn")` 吻合） |
| 实现 | `crates/yuris-format/src/ypf.rs` |
| 参考 | shimamura-sakura/yuri `yuri.md`、arcusmaximus VNTranslationTools `Yuris/Notes.txt`（仅参考文档，未拷贝代码） |

> **勘误（2026-10-24，Mikage fork）**：本文旧版把 `name_size` 字节当成名字首字符
> （于是条目名多出 `$`/`%`/`9` 前缀）、把 `file_type` 当成 NUL 终止符，并把 0x20 处
> 的首条 `name_hash` 当成「用途 Unknown 的 index_prefix」，末条因此出现 −4 字节残差；
> 「bn 型 / se 型双布局」也是同一误读的结果（`file_type` 非 0 时名字后面不再是 0）。
> 以下为按公开格式重写的布局。

## Header（0x20 字节）

| Offset | 类型 | 说明 |
|---|---|---|
| 0x00 | char[4] | `YPF\0` |
| 0x04 | u32 | 版本（如 500） |
| 0x08 | u32 | 条目数 |
| 0x0C | u32 | header_size：>= 300 时 = 索引区结束的绝对偏移；< 300 时不含 header 自身 |
| 0x10 | u8[16] | 0 |

## Entry（紧接 header，共 file_count 条）

| 类型 | 字段 | 说明 |
|---|---|---|
| u32 | name_hash | 解密后（小写）名字的哈希：0.477 起 murmur2，0.265 起 CRC32，更早 Adler32 |
| u8 | name_size | 加密：`NAME_SIZE_MAP[v ^ 0xFF]`，表为恒等置换交换 (3,72) (17,25) (46,50) (6,53) (9,11) (12,16) (13,19) (21,27) (28,30) (32,35) (38,41) (44,47) |
| u8[name_size] | name | 每字节 XOR name_key（公开版 0xFF；样本 v500 为 0xC9），CP932，分隔符 `\` |
| u8 | file_type | 与扩展名相关的类型码 |
| u8 | compression | 0 = stored，1 = zlib |
| u32 | uncompressed_len | |
| u32 | stored_len | |
| u32 / u64 | offset | 数据绝对偏移；约 0.477 起为 u64 |
| u32 | data_hash | 未解压数据的哈希（未校验） |

## 推断

商业定制版的版本号不可信，实现不按版本号硬判：

- **偏移宽度**：先按版本猜，再试另一种；取「恰好走完索引区（结束于 header_size 或
  header_size+0x20）且所有数据区都落在文件内」的那一种。
- **name_key**：对 0..=255 逐个解密前 8 条名字，与 name_hash（三种算法、原名或小写名）
  比对；都不吻合时退回「扩展名前一个字节应为 `.`」的投票。

## 名字查找

查找时统一归一：`/` → `\`，ASCII 小写；CP932 双字节字符的尾字节（可落在 `A`..`Z`、`\`）
保持不变。
