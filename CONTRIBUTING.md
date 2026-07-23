# Contributing

Rusty Fleet uses stacked milestones so the repository can move in meaningful
vertical slices without turning every small edit into a workflow event.

Before contributing:

1. Read [AGENTS.md](AGENTS.md) and the
   [workflow](docs/WORKFLOW.md).
2. Confirm that the work fits the active milestone stack and its repository
   and path envelope.
3. Keep the feature lock inert unless the milestone explicitly changes
   activation.
4. Run the smallest focused test while editing.
5. Run `Quick` before a coherent local commit and `Standard` before milestone
   handoff.

Use descriptive commits for coherent internal layers. A commit may contain
multiple files and tests when they form one reviewable result. Avoid
file-by-file commits and lifecycle units that exist only to rename a symbol,
add one fixture, or repair a failure discovered inside the current milestone.

Report security issues through GitHub's private security-advisory flow rather
than a public issue. See [SECURITY.md](SECURITY.md).

By contributing, you agree that your contribution is licensed under
`AGPL-3.0-or-later`.
