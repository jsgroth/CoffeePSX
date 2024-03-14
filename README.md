# ps1-emu

Work-in-progress attempt at a PlayStation emulator. Barely anything is implemented right now, does not boot anything yet. Display is simply a dump of VRAM, interpreted as a 1024x512 grid of RGB555 pixels.

For now at least I am doing this as a standalone emulator instead of adding it to my multi-system emulator so that I can more easily experiment with different ways of handling rendering and parallelism.

To run a PS1 EXE (sideloaded after the BIOS is initialized):
```
cargo run --release -- -b <bios_path> -e <exe_path>
```

The `-t` flag enables TTY output, printed to stdout.

## Status

Implemented:
* The R3000-compatible CPU, minus I-cache and remotely accurate timing
* Most of the GPU, using a software rasterizer
* Part of the SPU (Sound Processing Unit), enough to get basic audio output
* Enough of the CD-ROM controller to make the BIOS think there's no disc in the drive
* Digital controllers (P1 only)
* Most of the hardware timers

Not yet implemented:
* More accurate CPU timing; currently hardcoded to 2 cycles per instruction
* CPU instruction cache
* GPU: 24-bit color mode and display area cropping
* SPU: Capture buffers, noise generator, pitch modulation, interrupts, DMA, reverb FIR filter
* Most CD-ROM functionality
* GTE (Geometry Transformation Engine)
* MDEC (Macroblock Decoder)
* Analog controllers and P2 inputs
* Memory cards
* Synchronization modes for dotclock and HBlank timers
