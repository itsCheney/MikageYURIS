# How scripts run

Code: `crates/yuris-vm`. The VM runs a game's compiled scripts directly:
it loads `yst%05d.ybn` files as they are needed and executes their command
groups. Most of the execution model was reversed from the v555 engine
(kemonomichi2) and checked against all of its scripts; the parts that were
not reversed (tasks, text, input) were modelled on what Erewhon's scripts
expect, and their module comments say so.

## Groups and the main loop

A script is a list of command groups (see
[../formats/ystb.md](../formats/ystb.md)). The engine's loop is

```c
while (!yield) { pc = obj->pc++; yield = handler[group[pc].command](); }
```

so the PC is taken and then advanced, and a jump writes its target group
index. A handler that returns non-zero gives up the frame (WAIT, a click
wait, an error). `GroupVm::run` does the same with a group budget and never
blocks: it returns `VmSuspend::Wait` when the script waits, `Complete` when
it ends and `Error` when it cannot go on.

**Command map.** Scripts index commands and parameters in the game's own
YSCM order. When a script loads, `commands.rs` rewrites both to the v555
order by name, so the handlers are written once. Commands that v555 does
not have become `0x80 | index` and take the unimplemented path.

**Unimplemented commands.** In strict mode (the default for tests) the VM
stops at a command it cannot run; otherwise it records an `Unsupported`
event and goes on. `yuris boot` lists them at the end of a run.

## Control flow

All control flow is in commands; expressions have no jumps.

- **GO** jumps to a label: its window holds the name, looked up in YSLB
  (murmur2 of the name), which gives a script number and a group. The name
  may be computed at run time. A jump into another script loads it.
- **GOSUB** evaluates its condition, pushes a frame (the return position and
  the IF and LOOP stack depths) and jumps. The group's `gparam` gives the
  frame's INT, FLT and STR slot counts; the arguments become `@53`, `@54`
  and `$55` of the new frame. A label id may be inlined in the tag bytes
  (`B3 * 0x100 + B2`), otherwise the name is evaluated and hashed.
- **RETURN** writes its values into the caller's `@60`, `@61` and `$62`
  (RINT*n* goes to `@60[n]`), pops the frame and drops the IF and LOOP
  entries opened inside it. Returning from the base frame ends the script.
- **IF** (3 windows) evaluates its condition and on false jumps to the
  group in the second window's `len` field, or the third's when that is 0.
  These are group numbers filled in by the compiler. It pushes the end
  target on the IF stack.
- **ELSE** (0x0b) with a condition is an else-if: true enters the block,
  false jumps to the next ELSE of the chain (its second window's `len`) or
  to the end. **IFBLEND** (0x2d) closes a block: it jumps to the end of the
  chain. **IFEND** pops the IF stack.
- **LOOP** pushes `{body, count, counter = 1}`; the exit target is the
  second window's `len`. **LOOPEND** loops while `counter < count`;
  **LOOPBREAK** and **LOOPCONTINUE** take a level count. A negative count
  loops until a break. `@48` reads the innermost counter, and LOOPBREAK
  also drops the IFs opened in the loop.
- **LET** (2 windows): the first is the target (a reference, with indices
  for an array element), the second the value. The target window's tag
  byte 3 is the compound operator: 0 `=`, 1 `+=`, 2 `-=`, 3 `*=`, 4 `/=`,
  5 `%=`, 6 `&=`, 7 `|=`, 8 `^=`.
- **LABEL** does nothing at run time; labels come from YSLB.
- **END** ends the script.

**Declarations.** Variables are declared by YSVR, which the VM applies at
boot (kinds 1 and 3) and when a script is first loaded (kind 2). The
`F_`, `G_` and `S_` declaration commands do nothing at run time (the
engine gives them a no-op or the default handler). Plain INT, FLT and STR
groups do run: they set the initial value, converted to the declared type,
and an array form resets the array to its declared bounds.

## Boot

The engine boots in this order: YSCM command table, variable descriptors,
YSVR initial values, look up the entry label by hash, create the main task,
load its script, run. `boot.rs` does the same from a `GameFs` (archives,
loose files, save folder, optional EVB overlay); the entry label is
`SYSTEM_START` unless the caller names another. The YSTB key is guessed
from the first script.

## Tasks

Erewhon runs most of its flow as tasks: the main script only runs the
per-frame effect and button tasks, and the scenario is started with
`TASK ID=... #=label`. Each task has its own PC, frames, IF and LOOP
stacks; variables are shared. In each frame every runnable task runs until
it waits or ends, in order of Z and then creation (the main script has
Z 0). See `tasks.rs` for TASK, TASKEND, TASKINFO and `TASK E=0`.

## Text

In Erewhon the engine draws no text itself (`TEXT OUTPUTMODE=0`). Text lines
compile to WORD groups (raw text), `_` groups (a printed value) and a
closing RETURNCODE that carries the line number. The engine collects them
into `$_TEXT` and a flag string `$_TEXT_FLAG`, and the game's own text task
draws them, waits for the click and restarts the writer. `text.rs`
describes the protocol, including how the writer yields between lines.

## Input

Button state variables count frames: 1, 2, 3, ... while held, -1, -2, ...
after release; `@_INP(code)` reads the same by button code. The player
feeds mouse, keys and the cursor through `GroupVm`'s input calls. Ids and
codes are in `input.rs`.

## MATH and MATH3D

MATH follows the manual (`math.rs`). MATH3D is not in the manual: it
projects a point through a camera and gives the screen position and scale
that Erewhon uses to place its sprites. It was read from Erewhon's exe (see
[reversing.md](reversing.md)); the formulas are in `math.rs`.

## Native commands

Commands that act on the screen, sound or windows (the CG family, FONT,
SOUND, WINDOW*, MOUSE) go to a `Native` implementation: the VM evaluates
the parameters, names them by the canonical YSCM parameter names, and for
the `*INFO` commands stores the answer into the `LET` variable.
`yuris-host` is the implementation, see [host.md](host.md).
