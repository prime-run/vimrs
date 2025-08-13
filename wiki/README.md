# Evremap Developer Wiki

A concise guide for new contributors to understand the internals of `evremap`.

Evremap is a Linux keyboard remapper that:
- Grabs exclusive access to an input device
- Computes an effective set of pressed keys from a mapping config
- Emits those keys to a virtual uinput device

See:
- Architecture: `wiki/architecture.md`
- Modules: `wiki/modules.md`
- Configuration & parsing: `wiki/configuration.md`
- Event pipeline & algorithms: `wiki/event_pipeline.md`
- CLI & runtime wiring: `wiki/cli.md`
- Development workflow: `wiki/development.md`
- Extending the system: `wiki/extending.md`

## Repository layout
- `Cargo.toml` — dependencies and crate metadata
- `src/main.rs` — entrypoints and CLI
- `src/deviceinfo.rs` — device discovery and selection
- `src/mapping.rs` — config parsing and mapping model
- `src/remapper.rs` — core remapping engine (event loop and transformations)
- `README.md` — end-user overview and examples
- `pixelbookgo.toml`, `test.toml` — sample configs
- `Makefile` — simple check/test/format targets
