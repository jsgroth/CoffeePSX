# ps1-emu

Work-in-progress attempt at a PlayStation emulator. Barely anything is implemented right now, does not boot anything yet.

For now at least I am doing this as a standalone emulator instead of adding it to my multi-system emulator so that I can more easily experiment with different ways of handling rendering.

To run a PS1 EXE (sideloaded after the BIOS is initialized):
```
cargo run --release -- -b <bios_path> -e <exe_path>
```
