# Expression bytecode

Code: `crates/yuris-script` (`insn.rs` decodes, `eval.rs` evaluates) and
`crates/yuris-value` (values and variables).

Each parameter window of a script (see
[../formats/ystb.md](../formats/ystb.md)) holds one expression in postfix
form. The expression layer has no jumps: all control flow is done by
commands (see [vm.md](vm.md)).

## Encoding

```text
instruction = op: u8, len: u16, operand: u8[len]
```

The engine's interpreter loop does exactly this: read the op, skip
`len + 3` bytes, call the handler for the op from a 256-entry table, until
the window's length is used up. Ops without a handler raise an error in the
engine, so the decoder turns any unknown op, or a known op with an
unexpected operand length, into `Insn::Unknown` and the evaluator refuses
it.

## Instructions

| Op | Operand | Meaning |
|---|---|---|
| `42` `57` `49` `4c` | i8, i16, i32, i64 | push an INT |
| `46` | f64 | push a FLT |
| `4d` | bytes | push a STR literal (see below) |
| `48` | prefix, u16 id | push a variable's value (element 0 of an array) |
| `56` | prefix, u16 id | push a variable reference |
| `76` | prefix, u16 id | push a variable reference (indexed and member forms) |
| `29` | u8 (always 0) | array element: the values above the nearest reference are its indices |
| `2b` `2d` `2a` `2f` `25` | none | `+` `-` `*` `/` `%` |
| `3d` `21` `5a` `53` `3e` `3c` | none | `==` `!=` `>=` `<=` `>` `<` |
| `41` `4f` `5e` | none | `&` `|` `^` (bitwise) |
| `26` `7c` | none | `&&` `||` |
| `52` | none | unary `-` |
| `73` `69` | none | `$()` to STR, `@()` to INT |
| `2c` | none | separator between list items; does nothing |

The ops are the ASCII characters of the operators where there is one
(`+` is 0x2b, `<` is 0x3c), which is how they were identified in the
official compiler.

**Variables.** The operand's first byte is the source prefix: `@` (0x40)
numeric, `$` (0x24) string, `#` (0x23) a label reference as in
`GO[#=es.ERIS]`; a backtick (0x60) also occurs and is not understood. The
id indexes the variable table (YSVR, see
[../formats/ysbin.md](../formats/ysbin.md)); the prefix is part of the
key, so `@5` and `$5` are different variables.

**Arrays.** `@A(2,3)` compiles to `56 @A`, the index expressions, `29`. The
evaluator keeps references on a stack of their own, so `29` takes every
value pushed since the latest reference as an index list, in dimension
order, and replaces them with the element. In a LET target window the final
`29` is not executed: the reference and its indices are the assignment
target.

**String literals.** The payload starts with a delimiter (normally `"`) and
the value is the bytes up to the next one. `\\`, `\n` and `\t` are escapes;
any other backslash is kept. A CP932 lead byte (0x81 to 0x9F, 0xE0 to
0xEF) takes the next byte with it, so a trail byte that equals `"` or `\`
does not end the string.

## Values

INT is i64 and FLT is f64; STR is a byte string in the game's code page,
converted only when shown. From the manual and the scripts:

- INT arithmetic wraps; INT with FLT gives FLT; storing a FLT into an INT
  variable truncates.
- `+` of two strings concatenates; mixing strings and numbers needs `$()` or
  `@()`, as the manual says, and is an error otherwise.
- Comparisons give 1 or 0. Strings compare only with `==` and `!=`.
- Integer division or modulo by zero is an error (the engine's behaviour is
  not known).

## System variables

Ids below 1000 include the engine's computed variables, read without
storage. The ids differ between engine versions: v480/v481 (Erewhon) start
the system table at id 10 (`@_TIME`), giving `$_TEXT` = 122,
`@_INP` = 144 and `@_MOUSE_L` = 147; later versions shift them.

| Variable | Meaning |
|---|---|
| `@1[1]` | engine clock in ms |
| `@48` | iteration count of the innermost LOOP (from 1; 0 outside a loop) |
| `@53` `@54` `$55` | the current GOSUB frame's INT, FLT and STR arguments |
| `@60` `@61` `$62` | the values the last RETURN gave back |
| `@114` | display colour depth: supplied by the virtual display (default 32) |
| `@115` `@116` | display-device size: virtual display defaults to the YSCF game size; host may override (v555 confirmed; not checked on v481) |
| `@133` `@138` | cursor position (the same ids in v481) |

Reading a frame variable past its count gives the type's default, and so
does reading a global array out of range; reading an undeclared variable is
an error. The input variables are described in `yuris-vm/src/input.rs`.

### Display-device evidence and portable policy

SkyChord v555 (`skychord.exe`) dispatches integer system-variable ids through
`0x5788a0 + id*4` in the reader at `0x447edc`. Entries 114, 115, 116 target
`0x4482f8`, `0x4482e8`, `0x4482d8`, reading globals `0x8353b0`,
`0x8353a8`, `0x8353ac`. Initialization at `0x4694de` obtains a desktop DC
and fills these via `GetDeviceCaps` indices 12 (BITSPIXEL), 8 (HORZRES),
10 (VERTRES). They describe the display device, not YSCF's canvas.

The portable runtime exposes a virtual display: bootstrap uses the YSCF canvas
size at 32 bpp, falling back to 1280x720 when configuration is absent/invalid.
This is a compatibility policy, not a claim that the original engine uses
YSCF for display metrics. `GroupVm::set_display_metrics` and
`Player::set_display_metrics` allow a host override. Surface resizing and
letterboxing do not update these values, resize the canvas, or alter cursor
coordinates. WINDOWINFO continues to report the virtual game window size.
Standalone VMs without bootstrap retain the legacy 1920x1080/32 default.
