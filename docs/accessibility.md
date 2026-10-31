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

The [playground](https://simonvanlierde.github.io/jsonvet/) runs jsonvet via
WebAssembly in the browser:

- Works with keyboard navigation: dialect switches are a radio group navigable
  with arrow keys, and correction notes and code snippets are focusable.
- Screen reader announcements use semantic roles (`role="radio"`, `role="tablist"`)
  and an `aria-live` verdict region that announces correction counts on edits.
- Colour schemes adapt to system preference (light and dark mode) with high-contrast
  ink, and problem counts and severity indicators accompany colour markings.

## Contact

If jsonvet is hard to use with your setup, email <code@duinlab.nl>. Say what
you tried, what you use (terminal, browser, screen reader, operating system) and what happened.
