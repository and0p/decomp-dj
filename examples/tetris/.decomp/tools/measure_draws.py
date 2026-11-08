"""
Measures how long the original takes to draw a screen with NMIs off: the CPU
cycles from a frame's NMI to waitForVBlankAndEnableNmi ($AA6B). The decomp's
busyUntil() calls use these totals.

    pip install py65
    python3 measure_draws.py tetris.nes ram1.bin [ram2.bin ...]

Each ram*.bin is the NES's 2 KB of RAM at the start of a frame (before its
NMI) in which the game goes on to draw a new screen, saved from an emulator.
The script finds the frame wait the game is in from the return addresses on
the stack, raises the NMI there and runs the original's code in py65 until it
reaches $AA6B. Reads from I/O give 0 and writes to I/O and the mapper are
ignored, which the drawing code doesn't depend on.

The NES has 29780.67 CPU cycles a frame: a total over that means the game
misses the next NMI (one frame lost), over twice that two, and so on.
"""
import os
import sys

from py65.devices.mpu6502 import MPU
from py65.memory import ObservableMemory

FRAME = 29780.67
STOP = 0xAA6B  # waitForVBlankAndEnableNmi
WAIT_LOOPS = {0xAA45: 0xAA4D, 0xAA2F: 0xAA37}  # updateAudioWaitForNmi..., its wait loop


def measure(prg, ram):
    mem = ObservableMemory(subject=[0] * 0x10000)
    for i in range(0x800):
        mem[i] = ram[i]
    for i in range(0x8000):
        mem[0x8000 + i] = prg[i]
    mem.subscribe_to_read(range(0x2000, 0x8000), lambda a: 0)
    mem.subscribe_to_write(range(0x2000, 0x10000), lambda a, v: True)
    mpu = MPU(memory=mem)

    # the innermost JSR to a frame wait on the stack is where the game is waiting
    sp, pc = 0x1FF, None
    while sp > 0x101:
        r = ram[sp - 1] | ram[sp] << 8
        if r < 0x8002 or prg[r - 2 - 0x8000] != 0x20:
            raise ValueError('no frame wait on the stack')
        sp -= 2
        callee = prg[r - 1 - 0x8000] | prg[r - 0x8000] << 8
        if callee in WAIT_LOOPS:
            pc = WAIT_LOOPS[callee]
            break
    if pc is None:
        raise ValueError('no frame wait on the stack')
    mpu.sp, mpu.pc = sp & 0xFF, pc

    def push(v):
        mem[0x100 + mpu.sp] = v
        mpu.sp = (mpu.sp - 1) & 0xFF

    push(pc >> 8), push(pc & 0xFF), push(mpu.p & ~0x10)
    mpu.pc = prg[0x7FFA] | prg[0x7FFB] << 8
    mpu.processorCycles = 7
    for _ in range(2_000_000):
        if mpu.pc == STOP:
            return mpu.processorCycles
        mpu.step()
    raise ValueError(f'did not reach ${STOP:04X}')


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    prg = open(sys.argv[1], 'rb').read()[16:16 + 0x8000]
    for path in sys.argv[2:]:
        c = measure(prg, bytearray(open(path, 'rb').read()))
        print(f'{os.path.basename(path):24s} {c:6d} cycles  {c / FRAME:.3f} frames')


if __name__ == '__main__':
    main()
