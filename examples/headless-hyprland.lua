-- A separate Hyprland 0.56 desktop. Copy this file before customizing it.
hl.monitor({ output = "", mode = "1280x720@30", position = "0x0", scale = 1 })
hl.config({
    input = { kb_layout = "us" },
    general = { gaps_in = 4, gaps_out = 8, border_size = 2 },
    animations = { enabled = true },
})
-- Alt bindings avoid macOS's reserved Command shortcuts.
hl.bind("ALT + RETURN", hl.dsp.exec_cmd("kitty"))
hl.bind("ALT + Q", hl.dsp.window.close())
for workspace = 1, 5 do
    hl.bind("ALT + " .. workspace, hl.dsp.focus({ workspace = workspace }))
    hl.bind("ALT + SHIFT + " .. workspace, hl.dsp.window.move({ workspace = workspace }))
end
hl.on("hyprland.start", function()
    hl.exec_cmd("kitty")
end)
