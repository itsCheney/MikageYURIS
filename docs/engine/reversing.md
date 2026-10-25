# Reversing notes

What was read from where, so the next person can check it or go further.
No engine code is copied into this repository; only facts are taken.

## Sources

- **The YU-RIS SDK** (0.495): the HTML manual for commands, parameters
  and system variables, and the compiler `YSCom.exe`, from which the
  bytecode ops, the YSTB layout and the encryption key derivation were
  read. The v480 compiler's dictionary (`.ycd`) has the system variable
  table in the order v480/v481 engines use.
- **kemonomichi2.exe** (AnimalTrailGirlishSquare 2, engine v555, YPF 500,
  PE32 x86). Upstream (Yuris-Tyn) decompiled it with Ghidra and checked the
  results against all 302 of its scripts; the execution model in
  [vm.md](vm.md) comes from there.
- **Erewhon's exe** (engine v481). Read statically with capstone and pefile
  (a Python venv with `pip install capstone pefile` is enough) for MATH3D
  and CGACT TEXT.
- **The scripts themselves.** Most semantics that were not reversed were
  fixed by what the scripts need: a wrong model shows up as a script that
  stalls, reads out of range or draws the wrong thing. `yuris boot` reports
  where a run stopped and the task call stacks.

## Erewhon (v481)

- The command handler table is at 0x7ad8a0, 119 slots in YSCM order. It is
  filled at run time by the code at 0x45ef38, so a static read has to
  follow those stores rather than look for a table in the data.
  - MATH 0x448a44, MATH3D 0x449080, CGACT 0x42bccc.
- Handlers read their parameters through a slot index: the byte at
  0x684da0 + slot gives the position, and integer values sit in the tables
  at 0x684a60 and 0x684200. For CGACT: SET is slot 9, BNO slot 62, TEXT
  slot 63 (byte 0x684ddf).
- The glyph routine CGACT TEXT calls is 0x4056a0; its loop over the SET
  offsets gives the outline and shadow rule in [host.md](host.md).

## kemonomichi2 (v555)

Ghidra names, as in upstream's notes.

| What | Address |
|---|---|
| main loop (`pc = obj->pc++; yield = handler[...]()`) | FUN_0040449c |
| script loader (sections, XOR, group tables) | FUN_00450dfd |
| command handler table / its initialiser | DAT_0078b020 / FUN_0046305c |
| parameter evaluator (slot map at DAT_006624a0) | FUN_004253cd |
| expression interpreter / op table / its initialiser | FUN_00420acc / DAT_007e1660 / FUN_0046a21c |
| string literal op 0x4d | 00420cb8 |
| GO, GOSUB, RETURN | 0044272c, 004428c0, 0044b418 |
| IF, ELSE, LET | 004431ec, 0043d34c, 00443808 |
| LOOP, LOOPBREAK, LOOPCONTINUE, LOOPEND | 00445a00, 00445b9c, 00445c54, 00445ce8 |
| system variable reads (INT; `switch` on the id) | 00447ebc |
| boot chain | FUN_0046b63c |
| YSVR load / apply | FUN_0046b7a0 / FUN_00451348 |
| YSLB load / label lookup (murmur2) | FUN_00463c7c / FUN_0045124c |
| LOAD (YSSD blocks) | FUN_00444648 |
| CG | 0x423864 |
| TASK, TASKINFO | 0044fe40, 00451838 |
| TEXT | 00452888 |

The engine copies the string `revsiruy` ("yurisver" reversed) into a
buffer during start-up, but the CRC call that makes the YSTB key was not
found; the key is recovered from the data instead (see
[../formats/ystb.md](../formats/ystb.md)).

## Open questions

- The task scheduler's real order and the text handlers (WORD, RETURNCODE)
  were modelled on Erewhon's scripts, not reversed.
- The sound mixer's volume and pan curves.
- CGACT TRANSITION, masks and movies.
- The engine's layout of layer blocks and of the `SAVE MEM=99999` block in
  save files (ours are our own; see the YSSD section of `formats/ysbin.md`).
- System variable ids above the v481 table in other versions.
