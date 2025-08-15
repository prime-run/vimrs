# Chord Remap Suppression

This page documents the behavior of remapped key chords when one of the inputs is released.

## Summary

- When a Remap chord matches (e.g., `Alt+F -> '-'`), the output is emitted as long as the chord is held.
- If any input of that chord is released, the chord output stops immediately.
- Any remaining non‑modifier base keys from that chord are suppressed until they are physically released.
- Suppressed keys produce no output (including repeats) and do not pass through.

This prevents stray base characters (like a bare `f`) leaking after a chord breaks.

## Examples

- Alt+F -> '-':
  - Press Alt+F ⇒ `-` is pressed.
  - Release Alt while holding F ⇒ `-` is released; nothing is emitted; `F` is suppressed until released. No stray `f`.
  - Release F ⇒ suppression ends.

- Alt+F -> '-', Alt+A -> '+':
  - Hold Alt, press F ⇒ `-` is pressed.
  - Release F (Alt still held) ⇒ `-` is released.
  - Press A (Alt still held) ⇒ `+` is pressed.

- Re-pressing a modifier while a non‑modifier is suppressed:
  - If F is suppressed (from breaking Alt+F) and you press Alt again while still holding F, nothing is emitted until F is released. Once F is released, subsequent chords can match as usual.

## Repeats

- While a key is suppressed, its repeat events are swallowed (no output).
- Repeats for active chord outputs behave as before.

## DualRole Interaction

- DualRole timing/tap detection is unchanged and independent of this feature.
- Only Remap chords are affected by suppression.

## Configuration Impact

- No changes needed in configuration files. Existing `remap` entries work with the new semantics.
- Modifiers (`Alt`, `Ctrl`, `Shift`, `Meta`, `Fn`) are not suppressed; only non‑modifier inputs from a broken chord are suppressed.

## Implementation Notes (for developers)

- The suppression policy is implemented in `src/remapper.rs`:
  - `suppressed_until_released`: per-key suppression set.
  - `active_remaps`: tracks the inputs/outputs of the currently active chord(s).
  - On release of any chord input, the chord is deactivated and remaining non‑modifier inputs still held are added to `suppressed_until_released`.
  - `compute_keys()` ignores suppressed keys prior to applying DualRole/Remap passes.
  - Repeat events for suppressed keys are swallowed.

## Rationale

- Keeps the system purely state-driven (no timers), consistent with the set-diff pipeline.
- Eliminates spurious base key emission when chords break.
- Supports smooth switching between chords under the same modifier (e.g., Alt+F → Alt+A).
