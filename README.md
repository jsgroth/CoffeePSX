# ps1-emu

Work-in-progress attempt at a PlayStation emulator. Many games do not boot, and performance in 3D games is quite poor. Currently CLI-only, no GUI.

Currently standalone rather than being an additional backend in [jgenesis](https://github.com/jsgroth/jgenesis) in order to enable easier experimentation for rendering and parallelism, since this is the first console I've emulated that supports 3D graphics (and a double-digit MHz main CPU).

## Status

Implemented:
* The R3000-compatible CPU, minus remotely accurate timing
* The GTE
* The GPU, with a (very very slow) software rasterizer
* Most of the SPU
* Most of the CD-ROM controller
* Partial MDEC functionality (15bpp/24bpp only, assumes output is always read via DMA1)
* Digital controllers, P1 only
* Memory card, port 1 only and shared across all games
* Most of the hardware timers

Not yet implemented:
* Accurate CPU timing; currently hardcoded to 2 cycles per instruction with no bus access delays
* DMA timings and GPU draw timings; right now all DMAs and GPU commands finish instantly from software's perspective
* PAL display and video timings; only NTSC is supported right now
* SPU: Capture buffers, noise generator, pitch modulation, reverb FIR filter
* Some CD-ROM functionality including infrequently used commands, audio report interrupts, and non-standard CD-XA audio modes
* MDEC 4bpp/8bpp modes
  * Even 15bpp/24bpp MDEC does not work properly in some games, possibly timing-related
* Analog controllers and P2 inputs
* More flexible memory card implementation (e.g. an option for whether to share across games or give each game its own emulated card)
* Interrupts and synchronization modes for dot clock and HBlank timers

## Build Dependencies (Linux)

This project uses the [cpal](https://crates.io/crates/cpal) crate for audio, which on Linux requires [ALSA](https://www.alsa-project.org/wiki/Main_Page) to build:

```
sudo apt install libasound2-dev
```

## Build & Run

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
* Step to Next Frame: N key
* Toggle 2x Prescaling: / key (Forward Slash)
* Toggle Bilinear Interpolation: ; key (Semicolon)
* Toggle VRAM view: ' key (Quote)
* Toggle Vertical Overscan Cropping: . key (Period)
* Exit: Esc key
