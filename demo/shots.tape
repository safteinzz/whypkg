# Every still in readme-assets/, rendered from the staged fake machine.
#
#   ./demo/stage.sh && vhs demo/shots.tape      (run from the repo root)
#
# The setup block is hidden, so a frame only ever holds the tool: no
# multiplexer bar, no scrollback, nothing to crop afterwards.
#
# 1180x620 is 112 columns by 28 rows. A browser row is 107 columns wide, so
# anything wider just adds dead space to the right of every description. The
# report is half that wide and lives in report.tape, in a frame of its own.
#
# Ctrl keys must be lowercase here: `Ctrl+G` is Ctrl+Shift+G, which the
# browser does not bind, and the frame silently comes out unchanged.
#
# Every Screenshot is followed by a Sleep. VHS grabs the buffer a beat after the
# command, so without one the next keypress lands first and the picture is of
# the screen after it - which is how a dossier shot once came out as the graph.

Output demo/home/.shots.gif

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1180
Set Height 620
Set Padding 24
Set TypingSpeed 60ms

Hide
Type "demo/enter.sh" Enter
Sleep 2s
Ctrl+l
Show

# ── the list you land on ────────────────────────────────────────────────────
Type "whypkg" Enter
Sleep 3s
Screenshot readme-assets/browse.png
Sleep 1s

# ── found by what it is, not what it is called ──────────────────────────────
Type "element"
Sleep 2s
Screenshot readme-assets/search.png
Sleep 1s
Backspace 7

# ── the answer ──────────────────────────────────────────────────────────────
Type "libllvm"
Sleep 1s
Enter
Sleep 2s
Screenshot readme-assets/dossier.png
Sleep 1s

# ── the neighbourhood ───────────────────────────────────────────────────────
Ctrl+g
Sleep 2s
Screenshot readme-assets/graph.png
Sleep 1s

Hide
Escape
Escape
Escape
Sleep 500ms
Type "exit" Enter
Sleep 500ms
