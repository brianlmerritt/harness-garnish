# Harness Garnish repository instructions

## Important!
- Do not drift from the product design without checking with the user
- Before and after each phase, check the product design and clarify any issues

## Project specific detailed requirements
- The user manages Git branches and commits for this repository.
- Do not create, switch, rename, or delete branches or worktrees.
- Do not commit, amend, rebase, merge, fetch, pull, push, open pull requests, change remotes, or update submodule pointers.
- Read-only Git inspection is allowed. Preserve all user changes and never run destructive Git commands.
- Keep canonical design decisions in `docs/decisions/`; supersede an ADR rather than silently reversing it.
- Do not claim a phase or capability is complete without the machine evidence defined in `docs/mvp-acceptance.md` or the applicable later-phase acceptance plan.
- Tests must not consume provider quota or API budget unless they are explicitly labelled and the user opts in, but do tell the user when provider quota or API budget tests are needed to sign off any given phase or development.
- Never store secrets or private chain-of-thought in the repository, database fixtures, logs, or run artifacts.

