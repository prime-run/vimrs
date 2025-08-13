# Development Workflow

## Prereqs

- Rust toolchain (edition 2021)
- libevdev headers and pkg-config (see `README.md` build section)
- Access to `/dev/input/event*` and `/dev/uinput` (root or appropriate udev rules / group membership)

## Build, check, format

- Check: `cargo check` or `make check`
- Build: `cargo build` / `cargo build --release`
- Format: `cargo +nightly fmt` or `make fmt` (uses `./.rustfmt.toml`)

## Run

- Typical remap run (needs permissions):

  ```bash
  sudo target/release/evremap remap /path/to/config.toml
  ```

- Debug key events without remapping:

  ```bash
  evremap debug-events --device-name "..." [--phys ...]
  ```

- List devices and keys:

  ```bash
  sudo evremap list-devices
  evremap list-keys  # printing currently commented in `src/main.rs::list_keys`
  ```

## Logging

- Default level is `Info`. Set environment:

  ```bash
  EVREMAP_LOG=trace EVREMAP_LOG_STYLE=always evremap ...
  ```

- Relevant logs emitted from `src/remapper.rs` (trace for IN/OUT events) and device acquisition in `src/main.rs`.

## Local test config

- Example configs: `pixelbookgo.toml`, `test.toml` (for dev experiments)
- `make test` runs `./target/debug/evremap remap ./test.toml` with a 20s timeout; ensure the binary exists and you have device permissions.

## System integration

- Example unit: `./evremap.service` (systemd). Adjust binary/config paths.
- Alternative init examples: see `README.md` (Runit, OpenRC).

## Code navigation

- Entrypoint: `src/main.rs`
- Core engine: `src/remapper.rs`
- Config & model: `src/mapping.rs`
- Device discovery: `src/deviceinfo.rs`

## Conventions

- Error handling: `anyhow::*` in application layer; typed error only for config parse (`ConfigError`).
- Logging: `log` facade with `env_logger`.
- Style: follow `rustfmt` config. Keep modules small and responsibilities narrowly defined.
