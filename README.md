# ps1-emu

Work-in-progress attempt at a PlayStation emulator. Some games are fully playable, but some do not boot or have major issues, and the emulator is missing essential features like memory card management and proper handling of multi-disc games.

## Status

Implemented:
* The R3000-compatible CPU
  * Currently implemented using a pure interpreter; performance could be a lot better
* The GTE
* The GPU, with both software and hardware rasterizers
  * Hardware rasterizer uses wgpu with native extensions; should work on Vulkan, DirectX 12, and Metal (has only been tested on Vulkan)
  * Hardware rasterizer supports 24bpp color rendering and higher resolutions up to 16x native
  * Supports basic PGXP (Parallel/Precision Geometry Transform Pipeline), which reduces model wobble and texture warping in many 3D games
    * CPU mode is not yet implemented so the PGXP implementation is not compatible with some games (e.g. Spyro series, Metal Gear Solid, Resident Evil 3, Tony Hawk's Pro Skater series)
* The SPU
* Most of the CD-ROM controller
* The MDEC
* The hardware timers
* Digital controllers, P1 only
* Memory card, port 1 only

Not yet implemented:
* Analog controllers and P2 inputs
* Configurable inputs and gamepad support
* More flexible memory card implementation (e.g. an option for whether to share across games or give each game its own emulated card)
  * Also a memory card manager
* Additional graphical enhancements for the hardware rasterizer (e.g. PGXP CPU mode, texture filtering)
* More accurate timings for DMA/GPU/MDEC; some games that depend on DMA timing work but timings are quite inaccurate right now
* Some CD-ROM functionality including disc change, infrequently used commands, and 8-bit CD-XA audio
  * There are possibly no games that use 8-bit CD-XA audio samples?
* Accurate timing for memory writes (i.e. implementing the CPU write queue)
  * It seems like maybe nothing depends on this?

## Software Rasterizer AVX2 Dependency

The software rasterizer makes very heavy use of x86_64 [AVX2](https://en.wikipedia.org/wiki/Advanced_Vector_Extensions#Advanced_Vector_Extensions_2) instructions. These have been supported in Intel CPUs since Haswell (4th gen i3/i5/i7) and AMD CPUs since Bulldozer (FX-41xx/61xx/81xx). There is a fallback rasterizer that does not use any x86_64 intrinsics but it is extremely slow and will probably not run 3D games at full speed.

The hardware rasterizer has no such dependency.

## Build Dependencies

This project uses [SDL2](https://www.libsdl.org/) for audio.

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

PS1 EXE files, CUE/BIN disc images, and CHD disc images are supported.

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
* Use hardware rasterizer: 0 key
* Use software rasterizer: - key (Minus)
* Decrease resolution scale: [ key (Left square bracket)
* Increase resolution scale: ] key (Right square bracket)
* Toggle VRAM view: ' key (Quote)
* Exit: Esc key

## Screenshot

![Screenshot from 2024-03-30 01-09-14](https://github.com/jsgroth/ps1-emu/assets/1137683/99c35745-31b0-4a1b-8733-321bc8a4a372)
