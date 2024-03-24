# ps1-core

Emulation backend for the PlayStation, Sony's first gaming console.

This console contains the following components that need to be emulated:
* R3000-compatible CPU (MIPS I) clocked at 33.8688 MHz with a 4KB instruction cache
* GPU (Graphics Processing Unit)
  * 1MB of VRAM for storing frame buffers, textures, and CLUTs (color lookup tables)
  * Can draw lines, triangles, and rectangles into a 2D frame buffer in VRAM
    * Polygons can be rendered using flat shading, Gouraud shading, or texture mapping
    * Textures can be 15-bit (raw RGB555 colors) or 4-bit/8-bit (indices into a CLUT)
    * Texture mapping can use raw texture pixels or color modulation
    * Supports a semi-transparency effect to automatically blend colors drawn on top of each other
    * Supports a dithering effect to reduce color banding of draw output
  * Supports a variety of frame buffer resolutions ranging from 256x240p to 640x480i
  * Supports frame buffer color depth of 15-bit (RGB555) or 24-bit (RGB888), although draw commands only support 15-bit color
* SPU (Sound Processing Unit), a 24-channel ADPCM chip
  * 512KB of sound RAM for storing ADPCM samples and the reverb buffer
    * First 4KB is reserved for capture buffers
  * ADPCM format contains 28 compressed PCM samples per 16-byte block
  * Outputs at 44100 Hz
  * Hardware support for ADSR envelopes, stereo sweep envelopes, reverb, and pitch modulation
* GTE (Geometry Transformation Engine), a 3D math coprocessor that can perform hardware-accelerated vector and matrix operations
* MDEC (Macroblock Decoder), a hardware image decompressor that supports the custom Macroblock format (somewhat similar to JPEG)
* CD-ROM controller containing a 2x CD-ROM drive, a Motorola 68HC05 CPU, and a DSP
  * CD-ROM format supports up to roughly 650MB of storage on a 74-minute disc (most common at the time)
  * 2x drive supports data transfer speeds up to 300KB per second for sequential reads
  * Supports audio playback of both CD audio tracks and CD-XA sectors containing ADPCM samples
  * The CD controller as a whole can be emulated as a single component, so it is not necessary to directly emulate the 68HC05 or the DSP
* Hardware timers tracking the system clock, the GPU dot clock, and GPU horizontal retraces (i.e. a scanline counter)
* 512KB BIOS ROM containing boot code and a system kernel
  * The BIOS itself is not emulated - a copy of a BIOS ROM is required for the emulator to function
* 2MB of main RAM
  * First 64KB is reserved for use by the BIOS kernel
* 1KB of very fast scratchpad RAM
  * The scratchpad is technically the CPU's data cache, but it is not wired up in a way that it can actually be used as a MIPS data cache
* Support for digital and analog controllers
* Support for 128KB memory cards for storing save data
