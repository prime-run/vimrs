---@meta

---@class PrimemapMode
local PrimemapMode = {}

---Define a dual-role key: when held, emits hold modifiers; when tapped quickly, taps keys in `tap`.
---@param input string @ e.g. "KEY_CAPSLOCK" or "a" or "<C-s>"
---@param hold string|string[] @ e.g. "KEY_LEFTCTRL" or {"KEY_LEFTCTRL"}
---@param tap string|string[] @ e.g. "KEY_ESC" or {"KEY_ESC"}
function PrimemapMode.dual_role(input, hold, tap) end

---Simple remap; input can be a chord spec or list; output can be single key or list.
---@param input string|string[] @ e.g. "<M-k>" or {"KEY_LEFTALT","KEY_K"}
---@param output string|string[] @ e.g. "KEY_UP" or {"KEY_LEFTSHIFT","KEY_9"}
function PrimemapMode.remap(input, output) end

---Sequence (macro) remap. Sequence uses tokens like "<C-[><Esc>" or "<Home><S-End><C-c>".
---@param input string @ e.g. "<C-s>"
---@param sequence string @ e.g. "<C-[><Esc>"
function PrimemapMode.remap_seq(input, sequence) end

---Switch to another logical mode.
---@param input string @ e.g. "<M-v>"
---@param mode string @ e.g. "visual"
function PrimemapMode.switch(input, mode) end

---@class Primemap
local Primemap = {}

---Set device metadata (can be overridden via CLI flags).
---@param name string
---@param phys string|nil
function Primemap.device(name, phys) end

---Create or get a mode by name.
---@param name string
---@return PrimemapMode
function Primemap.mode(name) end

---Global helper to register a sequence remap, optionally specifying a mode name.
---@param mode string|nil @ nil means global/default
---@param input string
---@param sequence string
function Primemap.remap_seq(mode, input, sequence) end

---@type Primemap
primemap = Primemap
