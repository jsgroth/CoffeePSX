# ps1-emu

Work-in-progress attempt at a PlayStation emulator. Some games are fully playable, but some do not boot or have major issues, and the emulator is missing essential features like memory card management and proper handling of multi-disc games. Currently CLI-only, no GUI.

## Goals

My primary goals with this project are, in no particular order:
* Learn how the PS1 worked and how games utilized the hardware by emulating it
* Learn a bit about graphics programming via a practical application (emulating the PS1 GPU in hardware)
* Make an emulator that I'd personally be willing to use, even if it's not the best out there

## Status

Implemented:
* The R3000-compatible CPU
* The GTE
* The GPU, with both software and hardware rasterizers
  * Hardware rasterizer uses wgpu with native extensions; should work on Vulkan, DirectX 12, and Metal (has only been tested on Vulkan)
  * Hardware rasterizer supports 24bpp color rendering and higher resolutions up to 16x native
* The SPU
* Most of the CD-ROM controller
* The MDEC
* Digital controllers, P1 only
* Memory card, port 1 only and shared across all games
* The hardware timers

Not yet implemented:
* Accurate timing for memory writes (i.e. implementing the CPU write queue)
  * Though it seems like maybe nothing depends on this?
* More accurate timings for DMA/GPU/MDEC; some games that depend on DMA timing work but timings are quite inaccurate right now
* Some CD-ROM functionality including disc change, infrequently used commands, and 8-bit CD-XA audio
  * There are possibly no games that use 8-bit CD-XA audio samples?
* Analog controllers and P2 inputs
* More flexible memory card implementation (e.g. an option for whether to share across games or give each game its own emulated card)
* A GUI
* Additional graphical enhancements for the hardware rasterizer (e.g. sub-pixel vertex precision, texture filtering)

## Software Rasterizer AVX2 Dependency

The software rasterizer makes very heavy use of x86_64 [AVX2](https://en.wikipedia.org/wiki/Advanced_Vector_Extensions#Advanced_Vector_Extensions_2) instructions. These have been supported in Intel CPUs since Haswell (4th gen i3/i5/i7) and AMD CPUs since Bulldozer (FX-41xx/61xx/81xx). There is a fallback rasterizer that does not use any x86_64 intrinsics but it is extremely slow and will probably not run 3D games at full speed.

The hardware rasterizer has no such dependency.

## Build Dependencies

This project uses [SDL2](https://www.libsdl.org/) for audio.

Linux (Debian-based):
```
sudo apt install libsdl2-dev
```

Windows:
* https://github.com/libsdl-org/SDL/releases

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

Command line flags:
* `--hardware`: Boot using the hardware rasterizer
* `--no-vsync`: Disable VSync
* `-t`: Enable TTY output, printed to stdout

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
* Use AVX2 software rasterizer: - key (Minus)
* Use naive slow software rasterizer: = key (Equals)
* Decrease resolution scale: [ key (Left square bracket)
* Increase resolution scale: ] key (Right square bracket)
* Toggle Auto-Prescaling: / key (Forward Slash)
* Toggle Bilinear Interpolation: ; key (Semicolon)
* Toggle VRAM view: ' key (Quote)
* Toggle Vertical Overscan Cropping: . key (Period)
* Exit: Esc key

## Screenshot

![Screenshot from 2024-03-30 01-09-14](https://github.com/jsgroth/ps1-emu/assets/1137683/99c35745-31b0-4a1b-8733-321bc8a4a372)
