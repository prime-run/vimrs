# Extending Evremap

This guide explains how to extend the codebase for new mapping behaviors, mode logic,
or developer tooling. It references current structures and algorithms in `src/mapping.rs`
and `src/remapper.rs`.

## Adding a new Mapping kind

1. __Update the model__ in `src/mapping.rs`:
   - Add a new variant to `Mapping`:
     ```rust
     pub enum Mapping {
         DualRole { /* ... */ },
         Remap { /* ... */ },
         ModeSwitch { /* ... */ },
         // New variant
         NewKind { /* fields */ },
     }
     ```
   - Extend config structs and `serde` mappings to parse TOML into your new variant. If it can appear
     in `[modes.<name>]`, ensure you lift `mode=Some(name)`/`scope=Some(name)` consistently.
   - Update `MappingConfig::from_file()` layering to push your new variant in the correct order
     relative to other entries.

2. __Extend engine logic__ in `src/remapper.rs`:
   - Consider whether your variant participates in:
     - `lookup_dual_role_index(code)` — usually no unless it’s a new dual-role-like.
     - `lookup_mapping_index(code)` — decide tie-breaking vs. `Remap` and `ModeSwitch`.
       If it’s a chordal mapping, define how to compare chord sizes and whether it beats ModeSwitch on ties.
     - `compute_keys()` — if it transforms the effective key set (remove inputs, add outputs), add logic
       analogous to DualRole/Remap with `active_mode` gating.
     - `emit_repeat_for_active_remap()` — if it should repeat, implement repeat behavior.
   - Add a new `ActiveKind::NewKind` and propagate `active_remaps` bookkeeping.
   - Determine suppression semantics: when your mapping ends, which still-held physical inputs should be
     added to `suppressed_until_released` to avoid leakage?

3. __Output capabilities__ in `InputMapper::create_mapper()`:
   - Add any new output keys your mapping may emit to the uinput enablement set so the virtual device can
     synthesize them. Currently, outputs are collected from `DualRole.tap`, `DualRole.hold`, and `Remap.output`.

4. __TOML configuration__:
   - Document the new section in `wiki/architecture.md` and examples for developers (not user tutorials).
   - Ensure invalid keys surface `ConfigError` with helpful messages.

## Modifying mode behavior

- `active_mode` is an `Option<String>`, initialized to `Some("default")`.
- `ModeSwitch { scope }` eligibility requires `scope == active_mode` or `scope == None`.
- When a switch engages, its inputs are added to `suppressed_until_released`.

To alter behavior:
- __Global vs scoped switches__: adjust the scope check in `lookup_mapping_index()`.
- __Default mode__: change initialization in `RemapEngine::new()` and defaults in config parsing.
- __Persistent vs momentary modes__: currently modes are momentary chords; to persist, you could introduce
  a latch/toggle variant that sets `active_mode` until an explicit exit mapping clears it.

## Changing chord matching and priority

Chord matching is implemented in `lookup_mapping_index(code)`:
- `DualRole` exact-match wins when `input == code` and mode matches.
- Among `Remap` candidates whose `input` set is fully contained in the currently held physical keys,
  the largest chord wins.
- `ModeSwitch` beats `Remap` on chord-size ties.

Adjustments:
- __Tie-breaking__: reorder checks or introduce explicit priority for your new kind.
- __Partial-chord grace__: relax subset checks if you want near-miss matching (be careful with suppression).
- __Per-mode priority__: add a numeric priority field in the config and sort candidates accordingly.

## Suppression strategy

Suppression prevents key leakage when chords break.
- On release that ends a chord remap, any remaining still-held non-modifier inputs are added to
  `suppressed_until_released` and ignored until physical release.
- Suppression swallows press/repeat, but release removes from the set.

To change strategy:
- Modify where keys enter/exit `suppressed_until_released` in release/press paths.
- Consider allowing modifiers to bypass suppression for specific mappings.

## Emission ordering and modifiers

- Press modifiers first: `to_press.sort_by_key(|k| !is_modifier(*k))`.
- Release modifiers last: `to_release.sort_by_key(|k| is_modifier(*k))`.
- `is_modifier(KeyCode)` centralizes modifier membership.

If you add new modifier-like keys, extend `is_modifier()` accordingly.

## Repeats

- Active remap repeats are handled by `emit_repeat_for_active_remap()`.
- Fallback repeats consult `lookup_dual_role_index()` or `lookup_mapping_index()`.
- Suppressed keys never repeat.

To customize:
- Add per-mapping repeat rate or behavior and thread timing info into repeat emission.

## CLI and tooling

- Add subcommands in `src/main.rs` under `enum Command`. Wire args with `clap`.
- Add device inspection or tracing commands by reusing `DeviceInfo` and the event loop with alternate sinks.
- Respect logging conventions: `EVREMAP_LOG`, `EVREMAP_LOG_STYLE`.

## Device discovery

- `DeviceInfo::with_name(name, phys)` resolves devices, preferring `phys`.
- To support custom selection (e.g., by vendor/product), extend `DeviceInfo` and scanner logic in `src/deviceinfo.rs`.

## Testing and debugging

- Use the `DebugEvents` subcommand to print raw events.
- Unit-test config parsing by feeding TOML to `MappingConfig::from_file()` with temporary files.
- For engine logic, add targeted tests that simulate sequences of press/release/repeat and assert the
  emitted output events or the internal `output_keys` transitions.

## Style and invariants

- Keep all imports at file tops; avoid mid-file `use` statements.
- Maintain `mappings` as the single source of truth and keep `active_remaps` consistent with it.
- Always gate mapping application by `active_mode`.
- Batch writes between `SYN_REPORT`s and respect ordering around modifiers.
