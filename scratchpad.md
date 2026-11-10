This file is not for agents to alter unless prompted.

______NOTES________

- We should eventually split SPEC.md into named files. The work on symbols would be a great candidate for its own spec file.
- We should rename unreachable.txt to ignored.txt, as it should also cover blocks we just don't care about


_________ README DRAFT ____________

# decomp-dj
Accelerated dynamic analysis and playtesting for retro game decompilation

## Features

* 

## Why

Agentic workflows have accelerated the speed that developers can decompile retro games, but these projects often hit two major slowdowns: dynamic analysis and playtesting.

**Dynamic analysis** uses live, running gameplay 

**Manual playtesting** is 

Both of these tasks are major slowdowns.

Decomp projects often hit a wall after the first pass of static analysis when developers are required to repeatedly set up running emulators for situational dynamic analysis. Once dynamic analysis and decomp is mostly complete, developers then have to repeatedly play the decomp to test for bugs.

These snapshots present a number of problems. They are huge binary blobs that don't work well with version control. They contain copyrighted information and cannot be commited to a public repository.

## What games this works for
This tool will help you achieve frame-perfect parity for retro games that work with libretro cores, likely N64 or earlier. Newer systems might 

## What games this won't work for
Ports 