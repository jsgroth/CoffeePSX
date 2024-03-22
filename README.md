# ps1-emu

Work-in-progress attempt at a PlayStation emulator. Few games boot right now.

For now at least I am doing this as a standalone emulator instead of adding it to my multi-system emulator so that I can more easily experiment with different ways of handling rendering and parallelism.

To run a BIOS standalone:

```
cargo run --release -- -b <bios_path>
```

To run a PS1 EXE (sideloaded after the BIOS is initialized):
```
cargo run --release -- -b <bios_path> -e <exe_path>
```

To run a disc:
```
cargo run --release -- -b <bios_path> -d <disc_path>
```

CUE/BIN and CHD formats are supported. For CUE/BIN, `disc_path` should be the path to the CUE file.

The `-t` flag enables TTY output, printed to stdout.

## Key Bindings

Controller buttons:
* D-Pad: Arrow keys
* X: X key
* O: S key
* Square: Z key
* Triangle: A key
* L1: W key
* L2: Q key
* R1: E key
* R2: R key
* Start: Enter key
* Select: Right Shift key

Hotkeys:
* Save state: F5 key
* Load state: F6 key
* Pause: P key
* Toggle Bilinear Interpolation: ; key (Semicolon)
* Toggle VRAM view: ' key (Quote)
* Exit: Esc key

## Status

Implemented:
* The R3000-compatible CPU, minus I-cache and remotely accurate timing
* Part of the GTE (Geometry Transformation Engine), enough to get the BIOS to render the PS logo
* The GPU
* Part of the SPU (Sound Processing Unit), enough to get basic audio output
* Bare minimum CD-ROM functionality to get the simplest games to boot
* Bare minimum MDEC functionality (24bpp only, assumes output is always read via DMA1)
* Digital controllers (P1 only)
* Most of the hardware timers

Not yet implemented:
* More accurate CPU timing; currently hardcoded to 2 cycles per instruction
* CPU instruction cache
* A number of GTE operations
* SPU: Capture buffers, noise generator, pitch modulation, interrupts, DMA, reverb FIR filter
* A decent amount of CD-ROM functionality, including non-standard CD-XA ADPCM modes
* MDEC 4bpp/8bpp/15bpp modes
* Analog controllers and P2 inputs
* Memory cards
* Interrupts and synchronization modes for dotclock and HBlank timers
