# CoffeePSX

Work-in-progress attempt at a PlayStation emulator.

Some games are fully playable, but some do not boot or have major issues, and the emulator is missing some essential features like proper handling of multi-disc games and memory card management.

## Status

### Implemented

* CPU
  * Currently implemented using a pure interpreter; performance could be a lot better
* GTE (3D math coprocessor)
* GPU, with both software and hardware rasterizers
  * Hardware rasterizer uses [wgpu](https://wgpu.rs/) with native extensions; should work on Vulkan, DirectX 12, and Metal (has not been tested on MacOS/Metal)
  * Hardware rasterizer supports higher resolutions up to 16x native as well as 24bpp high color rendering
  * Supports basic PGXP (Parallel/Precision Geometry Transform Pipeline), which reduces model wobble and texture warping in many 3D games
    * "CPU mode" is not yet implemented so the PGXP implementation is not compatible with some games (e.g. Spyro series, Metal Gear Solid, Resident Evil 3, Tony Hawk's Pro Skater series)
* SPU (sound processor)
* Most of the CD-ROM controller
* Support for loading CUE/BIN disc images, CHD disc images, and PS1 EXE files
* MDEC (hardware image decompressor)
* Hardware timers
* NTSC/60Hz and PAL/50Hz support
* Digital and analog controller emulation
* Memory card, port 1 only

### Not Yet Implemented

* DualShock rumble support
* More flexible memory card implementation (e.g. an option for whether to share across games or give each game its own emulated card)
  * Also a memory card manager
* Additional graphical enhancements for the hardware rasterizer (e.g. PGXP CPU mode, texture filtering)
* More efficient CPU emulation (cached interpreter, recompiler)
* More accurate timings for DMA/GPU/MDEC; some games that depend on DMA timing work, but timings are quite inaccurate right now
* Some CD-ROM functionality including infrequently used commands and 8-bit CD-XA audio
  * There are possibly no games that use 8-bit CD-XA audio samples?
* Various emulator enhancements like increased disc drive speed, CPU overclocking, rewind, save state slots
* Accurate timing for memory writes (i.e. emulating the CPU write queue)
  * It seems like maybe nothing depends on this?

## Software Rasterizer AVX2 Dependency

The software rasterizer makes very heavy use of x86_64 [AVX2](https://en.wikipedia.org/wiki/Advanced_Vector_Extensions#Advanced_Vector_Extensions_2) instructions. These have been supported in Intel CPUs since Haswell (Q2 2013) and AMD CPUs since Excavator (Q2 2015). There is a fallback rasterizer that does not use any x86_64 intrinsics but it is extremely slow and will probably not run 3D games at full speed.

The hardware rasterizer has no such dependency.

## Build Dependencies

### Rust

This project requires the latest stable version of the [Rust toolchain](https://doc.rust-lang.org/book/ch01-01-installation.html) to build.

### SDL2

This project uses [SDL2](https://www.libsdl.org/) for audio and gamepad inputs.

Linux (Debian-based):
```shell
sudo apt install libsdl2-dev
```

MacOS:
```shell
brew install sdl2
```

Windows:
* https://github.com/libsdl-org/SDL/releases

## Build & Run

To run the GUI:
```shell
cargo run --release
```

To run in headless mode (no GUI window, will exit when the emulator window is closed):
```shell
cargo run --release -- --headless -f /path/to/file.cue
```

To build with fat LTOs (link time optimizations), which slightly improves performance and decreases binary size but increases compile time:
```shell
cargo build --profile release-lto
# Binaries located in target/release-lto/
```

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
* Fast forward: Tab key
* Pause: P key
* Step to next frame: N key
* Select to hardware rasterizer: 0 key
* Select software rasterizer: - key (Minus)
* Decrease resolution scale: [ key (Left square bracket)
* Increase resolution scale: ] key (Right square bracket)
* Toggle VRAM view: ' key (Quote)
* Exit: Esc key

## Screenshot

![Screenshot from 2024-03-30 01-09-14](https://github.com/jsgroth/ps1-emu/assets/1137683/99c35745-31b0-4a1b-8733-321bc8a4a372)
