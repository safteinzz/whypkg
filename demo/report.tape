# The `pending` report still. Its own tape because the report is 62 columns
# wide and 22 lines tall - in the browser's 112-column frame a third of the
# picture would be empty.
#
#   ./demo/stage.sh && vhs demo/report.tape     (run from the repo root)
#
# The Sleep after Screenshot is not decoration: VHS grabs the buffer a beat
# after the command, so the next keypress can otherwise land in the picture.

Output demo/home/.report.gif

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 720
Set Height 520
Set Padding 24
Set TypingSpeed 60ms

Hide
Type "demo/enter.sh" Enter
Sleep 2s
Ctrl+l
Show

Type "whypkg pending --quick" Enter
Sleep 3s
Screenshot readme-assets/pending.png
Sleep 1s

Hide
Type "exit" Enter
Sleep 500ms
