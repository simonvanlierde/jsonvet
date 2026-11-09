# Renders playground/og.png: `just social-card` runs it in the vhs container.
Output card.gif
Set Shell "bash"
Set FontSize 20
Set Width 1200
Set Height 630
Set Padding 32
Set Theme "Dracula"
Set TypingSpeed 0
Hide
Type "export PS1='$ ' PATH=/opt/bin:$PATH && cd /work && clear"
Enter
Show
Type "jsonvet staging.json5"
Enter
Sleep 2s
Screenshot card.png
