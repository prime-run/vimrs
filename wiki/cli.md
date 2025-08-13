# CLI & Runtime Wiring

CLI is defined in `src/main.rs` using `clap` derive. Entry enum: `Opt`.

## Subcommands
- `ListDevices`
  - Calls `deviceinfo::list_devices()` to print Name/Path/Phys for `/dev/input/event*`.
- `ListKeys`
  - Builds list via `EventCode::EV_KEY(...).iter()` and sorts.
  - Note: printing is currently commented out in `list_keys()`; enable as needed for dev tooling.
- `DebugEvents { device_name: String, phys: Option<String> }`
  - Resolves device with `get_device()` (no waiting).
  - `debug_events()` logs `EV_KEY` codes and `event.value` (0 release, 1 press, 2 repeat) using `log::info!`.
- `Remap { config_file, delay, device_name, phys, wait_for_device }`
  - Loads `MappingConfig::from_file`.
  - CLI overrides `device_name`/`phys` if provided.
  - Warns then sleeps `delay` seconds to let user release keys.
  - Acquires device via `get_device(name, phys, wait_for_device)`.
  - Creates mapper: `InputMapper::create_mapper(device.path, mappings)` and runs `run_mapper()`.

## Device acquisition (`get_device`)
- Try `DeviceInfo::with_name(name, phys)` once.
- If not found and `wait_for_device = true`, poll every 1s, backing off to 10s max (`Duration` backoff loop), until device appears. Logs debug errors during polling.

## Logging (`setup_logger`)
- `env_logger` with default level `Info`.
- Environment variables:
  - `EVREMAP_LOG` — e.g., `trace`, `debug`, `info`.
  - `EVREMAP_LOG_STYLE` — controls style (e.g., `always`).

## Process model
- Requires exclusive grab of the input device; typically needs root or proper udev permissions/group membership.
- Virtual output device is derived from the input device capabilities.
