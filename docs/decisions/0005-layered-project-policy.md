# ADR 0005: Layered project policy and autonomy

- Status: accepted
- Date: 2026-07-19

## Decision

Use managed policy, global defaults, account/agent profile, project policy, and task overrides in descending authority. A lower layer may narrow grants freely but may widen them only where the higher layer marks a field delegable.

Class 0 reads and Class 1 scoped writes/tests may run autonomously when the effective sandbox is attested secure. Git branch creation, commit, pull/fetch, push, PR, merge, and submodule updates are separate effects that each project configures. Default policy allows task worktree/branch creation and task-local commits but denies remote/integration actions. This repository overrides that default: the user manages branches and commits.

The agent cannot modify the policy, approval, or attestation that authorises its current run.

## Consequences

- Projects can range from manual Git control to narrowly autonomous integration.
- A container alone does not grant autonomy; effective policy and verified sandbox properties do.
- Policy provenance must be explainable field-by-field.

