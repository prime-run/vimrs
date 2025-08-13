# Architecture

Evremap is a single-binary tool that remaps Linux input events at the evdev layer.

Core flow:

1. CLI parses subcommand and options in `src/main.rs` (`Opt`).
2. Configuration is loaded via `MappingConfig::from_file()` in `src/mapping.rs`.
3. Device is discovered/selected through `DeviceInfo` in `src/deviceinfo.rs`.
4. `InputMapper` in `src/remapper.rs` grabs the device, creates a uinput device, runs the event loop.
5. Incoming key events are transformed into a computed set of output keys based on `Mapping` rules.

Key components:

- `MappingConfig`/`Mapping` — domain model for remapping rules and config file parsing (`src/mapping.rs`).
- `DeviceInfo` — discovery of `/dev/input/event*` devices; name/phys filtering (`src/deviceinfo.rs`).
- `InputMapper` — reads events, maintains state, computes effective keys, emits to virtual device (`src/remapper.rs`).
- CLI wiring, logging, waiting behavior — `src/main.rs`.

Data dependencies:

- `InputMapper::create_mapper()` enables all output key codes on the derived virtual device before starting, based on `Mapping` contents.
- `MappingConfig` preserves the order of `[[remap]]` entries as defined in the TOML; `[[dual_role]]` entries are processed prior to remap entries.

Runtime model:

- Exclusive grab of the real device (`GrabMode::Grab`).
- Event loop reads from the real device; non-key events are pass-through; key events are processed and emitted as synthetic events to the uinput device.
- Output device stays in sync using SYN_REPORT after each burst of writes.
