-- Neovim-like Lua config equivalent to the provided TOML

local default = primemap.mode("default")

primemap.device("Usb KeyBoard Usb KeyBoard", nil)

default.dual_role("KEY_CAPSLOCK", { "KEY_LEFTCTRL" }, { "KEY_ESC" })

default.remap("KEY_RIGHTALT", "KEY_BACKSPACE")

default.remap("<M-k>", "KEY_UP")
default.remap("<M-j>", "KEY_DOWN")
default.remap("<M-h>", "KEY_LEFT")
default.remap("<M-l>", "KEY_RIGHT")

default.remap("<M-f>", "KEY_MINUS")

default.remap("<M-[>", { "KEY_LEFTSHIFT", "KEY_9" })
default.remap("<M-]>", { "KEY_LEFTSHIFT", "KEY_0" })

default.switch("<M-n>", "visual")
default.switch("<M-v>", "visual")

local visual = primemap.mode("visual")

visual.remap("b", { "KEY_LEFTSHIFT", "KEY_LEFTCTRL", "KEY_LEFT" })

visual.remap("w", { "KEY_LEFTSHIFT", "KEY_LEFTCTRL", "KEY_RIGHT" })

visual.remap("d", "KEY_BACKSPACE")

visual.remap("y", "KEY_COPY")

visual.remap("p", "KEY_PASTE")

visual.switch("<Esc>", "default")
