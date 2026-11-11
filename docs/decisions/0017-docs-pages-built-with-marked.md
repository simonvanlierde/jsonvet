# 17. The docs pages are built from the Markdown with marked

Status: accepted (2026-11-11)

## Context

The docs in `docs/` were readable only on GitHub. The site should show them in the playground's
look, under its CSP (`script-src 'self'`, no inline script or style), and the Markdown must stay
the only source. Rendering CommonMark with GitHub's tables needs a Markdown parser, and the repo
had none.

## Options

Argued, not measured:

- **mdBook.** About two hours to set up, but its own theme, search and scripts, so the pages would
  look like every other mdBook and need a second CSP.
- **A hand-written renderer.** No dependency, but CommonMark and GitHub tables are a parser of
  their own, with its own tests and fuzzing.
- **marked, at build time only.** One dev-dependency with no dependencies of its own and no
  install scripts. Visitors get only the HTML it writes.

## Decision

`scripts/docs-site/build.mjs` renders the docs with marked into `playground/docs/`, which is not
committed. marked is pinned exactly (18.1.2), installed from a frozen lockfile, and an update waits
a week after its release (`minimumReleaseAge` in `scripts/docs-site/pnpm-workspace.yaml`).

## Consequences

A docs change needs no step besides the Markdown. The playground build now needs pnpm. Pages and
anchors follow GitHub's slug rules, so a link works on GitHub and on the site alike.

## Confirmation

`scripts/docs-site/build.test.mjs` checks every link and anchor, the CSP, unique ids and labels,
and the landmarks. CI runs it in the `vscode` job on every pull request, and `pages.yml` before a
deploy. axe-core found no violations on the docs pages on 2026-11-11.

## Revisit if

marked gains dependencies or install scripts, an advisory hits it, or the docs need more than
CommonMark and GitHub tables.
