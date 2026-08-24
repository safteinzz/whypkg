# The tour at the top of the README: what a new user would do, in order.
#
#   ./demo/stage.sh && vhs demo/demo.tape       (run from the repo root)
#
# One beat per feature, each finished before the next starts, results held for
# a couple of seconds and keystrokes hurried through. Ctrl keys are lowercase -
# `Ctrl+G` is Ctrl+Shift+G and the browser ignores it.

Output readme-assets/demo.gif

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1180
Set Height 620
Set Padding 24
Set TypingSpeed 55ms
Set Framerate 24

Hide
Type "demo/enter.sh" Enter
Sleep 2s
Ctrl+l
Show

# ── beat 1: every installed package, in one list ────────────────────────────
Type "whypkg" Enter
Sleep 2500ms

# ── beat 2: filter down to the 133 MB library nobody asked for ──────────────
Type "libllvm"
Sleep 1200ms
Enter
Sleep 3s

# ── beat 3: flip the list between what needs it and what it needs ───────────
Right
Sleep 2s
Left
Sleep 1200ms

# ── beat 4: the same package as a graph, and dig one hop deeper ─────────────
# The graph opens with the centre selected, and the centre is a column of one -
# so a side has to be chosen with Left/Right before Up/Down has anywhere to go.
Ctrl+g
Sleep 2500ms
Left
Sleep 700ms
Down
Sleep 700ms
Enter
Sleep 2500ms
Escape
Sleep 1s
Escape
Sleep 1s

# ── beat 5: back out to the whole list ──────────────────────────────────────
Escape
Backspace 7
Sleep 800ms

# ── beat 6: flatpak apps sit in the same list, found by their real name ─────
Tab
Sleep 400ms
Tab
Sleep 400ms
Tab
Sleep 2s
Enter
Sleep 1800ms
Right
Sleep 2200ms
Escape
Sleep 500ms
Escape
Sleep 800ms

# ── beat 7: the report - every upgrade waiting, and why it is here ──────────
Ctrl+l
Type "whypkg pending --quick" Enter
Sleep 4s
