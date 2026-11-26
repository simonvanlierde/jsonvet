# Accessibility

## Command-line interface

jsonvet is a command-line program. It should work with a screen reader, in a
terminal without colour, and in scripts and CI logs:

- Colour is never the only signal. A coloured message also says in words what it
  means, such as `error:` or `warning:`.
- It prints no colour when the [`NO_COLOR`](https://no-color.org) environment
  variable is set or when you pass `--color never`.
- When its output is not a terminal, such as a pipe, a file or a CI log, it prints
  plain lines, with no spinners, progress bars or cursor movement.
- It writes errors to standard error, and an error about a file names the place as
  `file:line:col`, so an editor or a screen reader can jump to it.

## Web playground

The [playground](https://simonvanlierde.com/jsonvet/) runs jsonvet via
WebAssembly in the browser:

- Works with keyboard navigation: dialect switches are a radio group navigable
  with arrow keys, and problem notes and code snippets are focusable.
- Screen reader announcements use semantic roles (`role="radio"`, `role="tablist"`)
  and an `aria-live` verdict region that says whether the text parses and how many problems a rule found. It is emptied when the verdict changes and filled again a second later, so a screen reader never reads the old verdict beside the new one.
- F8 and Shift+F8 jump to the next and previous problem.
- Colour schemes follow the system's light or dark mode, and a button in the header
  switches between them. Both use high-contrast ink, and problem counts and severity indicators accompany colour markings, and each dialect's count is written out ("1 error", "stops at 2:3"), not shown as bare numbers.

### Tested with

axe-core (WCAG 2.2 AA and best practice) at 400 px and 1440 px wide, in light and dark mode, on the first view, after pasting text, on each dialect and with the Fix and Format menus open, and by hand with the keyboard alone
(2026-11-16).

The docs pages: axe-core at 400 px and 1440 px wide, in light and dark mode, with no
violations (2026-11-11). Their build tests check landmarks, labels and unique ids on every page.
Press `/` to open docs search. Tab to a result and press Enter. For an exact rule ID, Enter
from the search field opens that rule's reference section. Escape closes the dialog and returns
focus. Without script the search button is hidden and the docs stay readable.
The matrix page: axe-core at 400 px and 1440 px wide, in light and dark mode, with a file's and a
parser's details open, with no violations (2026-11-15). After the changes for 3.5.1, axe-core (WCAG
2.0 to 2.2 A and AA, and best practice) again found no violation on the playground, a docs page,
the docs contents, the matrix page with a file's and a parser's details open, and the 404 page, at
400 px and 1440 px wide in light and dark mode (2026-11-15). The 2026-11-26 pass checked the
playground, reference, open search dialog and matrix, including open file details, with axe-core
in Chrome at the same widths in both themes. It found no reported WCAG A or AA violations.
The playground, search and matrix keyboard paths also passed in Firefox and WebKit.

### Known gaps

- No screen reader test yet, so the live announcements are unverified.
- The editor is a plain text area, so it has no code folding or bracket matching.

## Contact

If jsonvet is hard to use with your setup, email <code@duinlab.nl>. Say what
you tried, what you use (terminal, browser, screen reader, operating system) and what happened.
