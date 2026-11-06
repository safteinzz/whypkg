# Every still in readme-assets/, rendered from the staged fake machine.
#
#   ./render.sh shots
#
# The setup block is hidden, so a frame only ever holds the tool: no
# multiplexer bar, no scrollback, nothing to crop afterwards.
#
# 1180x620 is 112 columns by 28 rows. A browser row is 106 columns wide before
# its description, so anything wider is spent on descriptions nobody reads in a
# screenshot. The
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
# The list narrows while the query is typed, matching descriptions too.
Type "/Element"
Sleep 2s
Screenshot readme-assets/search.png
Sleep 1s
Escape

# ── the answer ──────────────────────────────────────────────────────────────
Type "/libllvm"
Enter
Sleep 500ms
Enter
Sleep 2s
Screenshot readme-assets/dossier.png
Sleep 1s

# ── the neighbourhood ───────────────────────────────────────────────────────
Ctrl+g
Sleep 2s
Screenshot readme-assets/graph.png
Sleep 1s
Escape
Sleep 300ms
Escape
Sleep 300ms

# ── what runs on it ─────────────────────────────────────────────────────────
Type "/nodejs"
Enter
Sleep 500ms
Enter
Sleep 2s
Screenshot readme-assets/uses.png
Sleep 1s

Hide
Type "q"
Sleep 500ms
Type "exit" Enter
Sleep 500ms
