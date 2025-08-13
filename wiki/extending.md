# Extending Evremap

This guide outlines where and how to add features or evolve the design.

## Add a new mapping type

- Files to touch:
  - `src/mapping.rs` — model + config parsing
  - `src/remapper.rs` — application logic
- Steps:
  1. Extend `Mapping` enum with a new variant.
  2. Add a corresponding `*Config` struct with `#[derive(Deserialize)]` and fields.
  3. Update `ConfigFile` with a `#[serde(default)]` list for the new table (e.g., `[[new_mapping]]`).
  4. Convert parsed config into `Mapping` in `MappingConfig::from_file` (preserve intended order).
  5. Update `InputMapper` to interpret the new variant in:
     - `compute_keys()` if it affects effective pressed keys
     - `lookup_mapping()` and/or `update_with_event()` if it needs special matching/behavior
     - `Repeat` handling if applicable

## Add “modes” support

- Current hints: commented `Mode` enum in `src/mapping.rs` and commented `mode` fields in structs.
- Approach:
  - Reintroduce `Mode` in `Mapping` and config structs.
  - Track current mode in `InputMapper` state.
  - Add transitions via dedicated mappings (e.g., a chord that switches mode) or via a new mapping variant.
  - Gate `lookup_mapping()` and `compute_keys()` on current mode.

## Make tap/hold timing configurable

- Location: `src/remapper.rs::timeval_diff()` and `update_with_event()` (200ms threshold).
- Options:
  - Add field to `MappingConfig` (global) or to `DualRole` entries (per-key) and carry it into `Mapping`.
  - Thread the value to `InputMapper` and consult it in tap detection.

## Improve device selection

- Current behavior: match by `name` and optional `phys` (`src/deviceinfo.rs`).
- Options:
  - Match on `input_id()` (vendor/product), `uniq()`, or other attributes.
  - Extend `DeviceInfo` and `ConfigFile` with new selectors and prefer the most specific.

## Support non-key events

- Currently: non-key events are passed through as-is (`run_mapper` branch for non-`EV_KEY`).
- To remap pointers/relative axes:
  - Define new mapping variants (e.g., for `EV_REL`, `EV_ABS`).
  - Enable corresponding capabilities on the virtual device before `UInputDevice::create_from_device()`.
  - Extend event processing to transform and emit those events.

## Robust config validation

- Add a validation pass in `MappingConfig::from_file()` (or separate function) to:
  - Detect unknown keys (already covered by `ConfigError`).
  - Warn about overshadowed remaps due to ordering.
  - Warn on conflicting dual-role definitions for the same `input`.

## Performance considerations

- `compute_keys()` and set cloning are simple and readable; fine for keyboard rates.
- If needed:
  - Use bitsets for `KeyCode` instead of `HashSet`.
  - Cache chord match results keyed by active modifier sets.

## Update the CLI

- Location: `src/main.rs::Opt` and `main()` match arms.
- Steps:
  - Add a new subcommand or flags to `Opt`.
  - Implement handler function and wire it into the `match` in `main()`.
  - Consider logging and error contexts (`anyhow::Context`).

## Extend modifier handling

- Location: `is_modifier()`, `modifiers_first/last` in `src/remapper.rs`.
- When introducing new modifier-like keys, update `is_modifier()` so ordering remains correct.

## Testing strategy (suggested)

- No dedicated test suite exists yet.
- Options:
  - Extract pure functions (e.g., `compute_keys`) into smaller units and test with synthetic `Mapping`/state.
  - Abstract input/output behind traits to simulate event streams.
  - Property-test remap invariants (e.g., modifiers pressed before non-modifiers on press; released last).

## Logging and tracing

- Use `log` macros consistently; `trace!` is used for event IN/OUT.
- Consider adding span-based tracing behind a feature flag without breaking the current `env_logger` default.

## Rustdoc-style snippets

### Where to extend the model (`src/mapping.rs`)

```rust
// Add a new variant and config conversion
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Mapping {
    DualRole { input: KeyCode, hold: Vec<KeyCode>, tap: Vec<KeyCode> },
    Remap    { input: HashSet<KeyCode>, output: HashSet<KeyCode> },
    // NewVariant { /* fields */ }, // <-- add here
}

impl MappingConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        // ...
        for dual in config_file.dual_role { mappings.push(dual.into()); }
        for remap in config_file.remap { mappings.push(remap.into()); }
        // for new_item in config_file.new_variant { mappings.push(new_item.into()); }
        Ok(Self { /* ... */ })
    }
}
```

### Where to extend behavior (`src/remapper.rs`)

```rust
impl crate::remapper::InputMapper {
    /// Compute effective pressed keys (extend with new variant behavior)
    fn compute_keys(&self) -> HashSet<KeyCode> {
        let mut keys: HashSet<KeyCode> = self.input_state.keys().cloned().collect();

        // First pass: DualRole
        for map in &self.mappings {
            if let Mapping::DualRole { input, hold, .. } = map {
                if keys.contains(input) {
                    keys.remove(input);
                    for h in hold { keys.insert(*h); }
                }
            }
            // if let Mapping::NewVariant { /* ... */ } = map { /* transform keys */ }
        }

        let mut keys_minus_remapped = keys.clone();
        // Second pass: Remap
        for map in &self.mappings {
            if let Mapping::Remap { input, output } = map {
                if input.is_subset(&keys_minus_remapped) {
                    for i in input { keys.remove(i); if !is_modifier(i) { keys_minus_remapped.remove(i); } }
                    for o in output { keys.insert(*o); if !is_modifier(o) { keys_minus_remapped.remove(o); } }
                }
            }
            // else if let Mapping::NewVariant { /* ... */ } = map { /* apply */ }
        }
        keys
    }
}
```

```rust
// Matching new variants when handling events (press/release/repeat)
pub fn update_with_event(&mut self, event: &InputEvent, code: KeyCode) -> Result<()> {
    match KeyEventType::from_value(event.value) {
        KeyEventType::Press => {
            self.input_state.insert(code, event.time);
            match self.lookup_mapping(code) {
                Some(_m) => { /* possibly match NewVariant */ self.compute_and_apply_keys(&event.time)?; }
                None => { self.cancel_pending_tap(); self.compute_and_apply_keys(&event.time)?; }
            }
        }
        KeyEventType::Repeat => { /* extend if NewVariant should repeat */ }
        KeyEventType::Release => { /* extend if NewVariant needs special release */ }
        _ => self.write_event_and_sync(event)?,
    }
    Ok(())
}
