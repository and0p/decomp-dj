# Plan: keeping cores and plugins from crashing or hanging dj

Status: agreed 2026-11-10; steps 1 (decoder plugins), 2 (comparison cores) and 3 (batch
commands) done 2026-11-10; step 4 (the GUI's own core) not started. Linked from [ROADMAP.md](ROADMAP.md#before-the-first-release).

## The problem

Cores, comparison cores and decoder plugins are native code loaded into dj's process. A
segfault, `abort()`, `exit()` or an endless loop in one takes the whole process with it,
and nothing in Rust can catch that. In the GUI that loses unsaved edits; in `dj run` or
`dj clean` it stops a batch with no report of what failed.

The only real protection is a separate process that dj watches and can kill. dj does that
in one place already: `dj mcp` runs each core in a `dj worker` child, over JSON lines on a
pipe (`player/src/mcp/worker.rs`, `mcp/play.rs`). Everything else runs in process.

What doesn't work, and why:

- **Catching a crash and going on in the same process:** after a fault in C code, the
  process's memory can't be trusted.
- **A thread with a timeout:** a hung thread can't be stopped (in Rust, or safely in C); it
  can only be abandoned, still using a CPU.
- **Checking a library once at load:** worth doing as a quick probe, but most crashes
  depend on the data or the game's state.

## Order

Least trusted and cheapest first.

1. **Decoder plugins decode in a worker.** Decoding runs once per report and is cached;
   its inputs and outputs are small. The parent never loads a plugin library at all: a
   `describe` call reads each plugin's tables in a worker when the project opens, and an
   `analyze` call decodes. A crash names the function it was decoding; a hang ends at a
   timeout scaled to the work. The built-in decoders stay in process.
2. **Comparison cores in the GUI get a worker.** They are the decomps under development,
   the most likely to crash (`dj_sync`, `dj_write`, `dj_check_parity` in the user's port).
   Frames and RAM through shared memory, one request per frame. A crash fades the
   comparison picture with "the comparison core crashed at frame N", as a parity failure
   does; baseline playback goes on; Restart starts a new worker.
3. **Batch commands play each chain in a worker.** `dj run`, `clean`, `hydrate`: a crash
   marks that segment failed (with the core's last log lines) and the other chains go on;
   no frame finished within a time limit (30 s) ends it. The command exits non-zero with
   the list.
4. **The baseline core in the GUI**, last: video, audio, recording, seeking and the
   snapshot cache all cross the boundary. Until then, in process:
   - a crash handler (SIGSEGV, SIGABRT; an unhandled-exception filter on Windows) writes
     `.local/crash.txt` (core, segment, frame, the last libretro call), shown on the next
     launch; and unsaved edits saved to `.local/` as they're made, so a crash loses none;
   - a watchdog thread: `retro_run` or a save or load not back after a few seconds shows
     "the core isn't responding" with Wait and Quit (keeping edits).

## Step 2 details: comparison cores

**Done 2026-11-10.** The GUI loads `RemoteComparison` (compare.rs): the same calls as
`Comparison`, run by a long-lived `compare` worker (`isolate::Worker`, `serve_session`; JSON
lines with bytes after them both ways).

- **The baseline handle.** `dj_sync`, `dj_write` and `dj_check_parity` get a `dj_baseline`
  whose `get_proc` hands out raw functions of the baseline core, by any name, which can't
  cross processes. The worker holds a copy of the baseline core (the *shadow*) and loads the
  baseline's save state into it before each of those calls (and before a RAM-compare check);
  the handle points to the shadow. Reads match the baseline; writes through it reach only the
  copy (docs/connectors.md now says a comparison core should only read).
- **What crosses per frame:** the frame's lines (`Action`s); for a check or a sync, the
  baseline's save state; for a `[WRITE]` that goes to `dj_write`, the state too; back, the
  picture when one is shown (RGB bytes after the reply line; shared memory wasn't needed).
  `Comparison` was split so its core-facing calls take data (`InputState`, `Action`s, an
  optional baseline) instead of the project, since the GUI's segments may hold unsaved edits
  the worker can't read from disk; `OpenSpec` is the resolved `[[compare]]` entry.
- **Failures:** a crash, a call over 10 s (loading 60 s, scaled by `DJ_WORKER_TIMEOUT_SCALE`)
  or a garbled reply ends the worker; the error says what it was doing ("crashed (signal 11,
  SIGSEGV) while checking parity (dj_check_parity)") and the GUI shows it as a stop, with
  **Compare > Restart comparison core**.
- **Found on the way (macOS), and fixed:** macOS's loader hands back an already-loaded
  library when the path asked for equals that library's *install name* (`otool -D`). A
  library linked with its own absolute path as install name (`cc -o /abs/lib.dylib`, as the
  test core is built) and loaded from a private copy first is then given back when the
  original is opened: two cores sharing one instance, the second's set-up overwriting the
  first's callbacks (seen as the comparison core's pictures going to the baseline's slot).
  Copies at different paths never collide, nor does the original loaded first; the real
  cores are safe as built (install names `mesen_libretro.dylib`, `@rpath/mgba_libretro.dylib`,
  `mupen64plus_next_libretro.dylib`, the Tetris decomp's relative path). Checked with a
  small C program over each load order (scratch, not kept). Fixes:
  - `Core::load_in` refuses a library whose `retro_run` is already another slot's, before
    calling any of its functions, with how to fix the build (`tests/compare.rs`
    `a_shared_library_instance_is_refused`);
  - the worker's shadow baseline loads from its own private copy when it's the comparison
    core's file, and loads first, so neither order nor install name matters there.
- **Checked:** `tests/compare.rs` `comparison_cores_in_workers`: save-state sync, `dj_sync`,
  `dj_write`, parity on every frame and a captured picture, a mismatch and a failed tick on
  the same frame as in process, a declined sync, a load error; and test cores that crash in
  `dj_run`, hang there, or crash in `dj_check_parity` (each named, the baseline plays on, the
  worker then says it has stopped). Tetris boot (534 frames, parity and the picture every
  frame): the decomp core and the identical Mesen core match on every frame through the
  worker, as in process, at the same speed (334 and 272 fps through the worker, 305 and 283
  in process).
- **Not done:** the CLI and MCP have no comparison cores yet, so nothing else uses the
  in-process `Comparison`, which the tests and the worker still use.

## Step 3 details: batch commands

**Done 2026-11-10**, as a supervisor rather than a worker protocol: `dj run`, `clean` and
`hydrate` start the same command as a child `dj` (`DJ_BATCH_CHILD`) and pass its output
through (`isolate::supervise`, called from `real_main`). Their code is unchanged apart from
two hooks:

- **Progress:** `Session::advance` reports each frame it starts (`isolate::batch_progress`,
  a short line on the child's stderr, which the supervisor reads and doesn't print), so a
  crash or hang is named to the frame. Per-frame reporting cost nothing measurable (Thwaite:
  661-698 frames/s supervised, 635-667 in process, run to run).
- **Leaving segments out:** a crash (a signal; on Windows an NTSTATUS exit code), or no
  progress for 30 s (60 s before the first frame, scaled by `DJ_WORKER_TIMEOUT_SCALE`), ends
  the child. The supervisor prints `crashed  <id> <title> at frame N: the core ...`, adds the
  segment to `DJ_BATCH_SKIP` and starts the command again; `play_all` and the `Hydrator`
  leave the segment out and mark those below it `blocked`. A child that crashes before any
  frame, or in a segment already left out, ends the command with an error. At the end: the
  list of segments left out, and exit code 1. `clean` and `hydrate` resume from the local
  database; `run` plays everything again (crashes are rare). A child that ends with its own
  error (hashes differ, say) passes its exit code through.
- `DJ_IN_PROCESS=1` runs without a supervisor (for a debugger).
- **Checked:** `tests/batch.rs`: a test core that crashes while Select is held and hangs while
  Start is (`TESTCORE_CRASH_ON_SELECT`, `TESTCORE_HANG_ON_START`); `dj clean` and `dj run`
  report both segments with their frames, mark the one below the crash `blocked`, clean or
  check the rest, and exit 1, in under 10 s.
- **Not covered:** the GUI's Clean all and Hydrate all run the `Hydrator` in the GUI's
  process: that's step 4.

## Shared pieces

- **One worker entry for both programs.** A hidden first argument (`__dj-worker <kind>`)
  that `dj` and `dj-gui` both check before anything else, so the GUI can start workers
  without a `dj` next to it. `DJ_WORKER_EXE` overrides the program (tests point it at
  `dj`).
- **Framing:** a JSON line per request and reply, with bulk bytes (code, flags, frames) as
  raw lengths after the line, or in shared memory once frames cross (step 2).
- **Progress and timeouts:** workers report progress lines (`{"at": ...}`); the parent reads
  on a thread and kills the child at a deadline, naming the last progress.
- **Cleaning up:** workers end when their stdin closes; on Linux also
  `PR_SET_PDEATHSIG`; a Windows job object (already on the roadmap for `dj mcp`).
- **Determinism** is unaffected: the worker runs the same code on the same inputs.

## Step 1 details: decoder plugins

**Done 2026-11-10.** `player/src/isolate.rs` (the shared layer: `run_if_worker`, called first by
`dj` and `dj-gui`; `call`, one request per worker process with a reader thread and a deadline;
crashes described by signal on Unix and exit code elsewhere), and in `decode.rs`
`RemoteDecoder` (what `describe` returned; `analyze` and `executed_bytes` through a worker),
`DecoderRef` (a built-in in process, or a plugin in a worker) and `worker_request` (the
worker's side, which uses `PluginDecoder`, still the in-process loader). The parent process
never loads a plugin library. `AnalyzedFunction` and its parts are now serde types.

Checked: `player/tests/decoder_plugins.rs` with a C plugin that crashes on one opcode
(the report keeps going, naming the function), one that loops forever (stopped at the
scaled deadline, about 1 s in the test), one that crashes inside `dj_decoders` (the project
won't open, naming the entry); and each example project with the reference plugin in a
worker: every summary, report, gaps, frontier, check-connector, annotations and function
output identical to the built-in decoders', apart from the note naming the plugin. `dj-gui`
answers worker requests itself.

Costs: a worker process per project open (`describe`) and per report decoded (cached after),
a few ms each; decoded functions cross as JSON (for an N64-sized map, some MB).

As first planned:


- Worker kind `decode`. Requests: `describe {library}` (the tables: cpu, max and fixed
  length, `mode_from_flags` for all 256 flag bytes, `alignment` for modes 0-15),
  `analyze {library, cpu, functions with their byte offsets, data}` + the code bytes, and
  `executed {library, cpu}` + code and flags (no-map gaps on a core without operand flags).
- Parent: `Decoders` holds what `describe` returned; `Decoders::analyze` and
  `Decoders::executed_bytes` run built-ins in process and plugins in a worker. A crash or
  timeout gives a report note ("the 65816 decoder (path) crashed decoding `func`
  (signal 11): ROM is counted in opcode bytes") instead of failing the report. A plugin that
  crashes in `describe` stops the project from opening, naming the `[[decoders]]` entry.
- Timeouts: `describe` 10 s; `analyze` 10 s + 2 s per MB of code + 1 s per 1,000 functions.
- Tests: a C plugin that crashes on one opcode and one that loops forever, in decode and in
  `dj_decoders`; the reference plugin through the worker giving the same results as the
  built-ins.
