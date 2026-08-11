# Delivery plan

Each milestone is an independently reviewable vertical slice. Narrow tests run
before its conventional commit; publication-level checks run again after the
work branch is integrated.

## Milestone 0 — reproducible foundation

Deliverables:

- install `rustup` and pin the current stable Rust toolchain after the cooling
  window, with `rustfmt` and `clippy`;
- align Node and pnpm with repository-local pins rather than the host defaults;
- scaffold Tauri 2 + Solid/TypeScript without example/demo features;
- set `Renamewright` as the product, window, executable, and package identity,
  using lowercase `renamewright` for machine-facing identifiers;
- create the Cargo workspace and crate boundaries from the architecture;
- add format, lint, test, dependency-policy, and build scripts;
- add Windows CI for the shell and Linux/Windows CI for Rust core tests.

Exit criteria: a minimal window starts on a prepared development machine, the
Rust workspace and frontend build from clean lockfiles, and all baseline gates
pass without filesystem write permissions beyond the app's own local state.

## Milestone 1 — planning vertical slice

Deliver one complete read-only flow:

- select or drop files;
- retain native paths in the backend registry;
- add a prefix rule;
- render a virtualised original/proposed preview;
- report unchanged, duplicate, and invalid-name diagnostics;
- export the validated plan as inspectable JSON without executing it.

Start with domain tests, then IPC contract tests, then the rendered interaction.
This slice validates the framework choice and the “preview is the product” model.

## Milestone 2 — journaled execution

- implement plan IDs, source generations, and pre-execution revalidation;
- implement two-phase temporary renames;
- append and synchronise journal records;
- stream progress and cancellation state;
- inject failures at every step and test rollback;
- recover or inspect incomplete journals at startup;
- project completed transactions and recovery state through the Rename Ledger;
- generate and execute validated Undo plans.

Exit criteria: swaps, cycles, case-only changes, interruption, stale sources, and
rollback failure all have behavioural tests and truthful UI states.

## Milestone 3 — modern rule pipeline

Add rule families in small batches, keeping each batch buildable:

1. literal/regex replacement and prefix/suffix templates;
2. sequence numbering with explicit scope and preview order;
3. extension, case, cleanup, and Unicode rules;
4. range/character-class rules and per-entry overrides;
5. versioned presets and CSV inspection export.

Every rule ships serialization migration coverage, property tests, keyboard
editing, and a trace explaining how the proposed name was formed.

## Milestone 4 — production interaction quality

- finalise the Workbench/Cobalt token system after a rendered design review;
- complete all interactive states, focus order, screen-reader naming, reduced
  motion, high contrast, and Korean/English message catalogs;
- validate large batches and define measured performance budgets;
- add recovery/activity views and diagnostic filtering;
- run real Windows browser-shell interaction and packaged smoke tests.

Temporary screenshots and traces remain outside the repository unless selected
as durable test fixtures.

## Milestone 5 — release hardening

- threat-model admission, IPC, journal, updater absence, and artifact boundaries;
- run Cargo advisory/license/source checks and frontend supply-chain checks;
- build reproducible Windows installer and portable artifacts;
- generate checksums and an SBOM;
- configure code signing only after the user supplies or selects a signing method;
- document recovery, data locations, limitations, and uninstall behaviour.

No deployment, signing, GitHub repository creation, or release publication occurs
without a separate explicit request.

## Immediate next implementation stage

Milestone 0 and the first read-only Milestone 1 slice are complete: native file
selection, backend-only path retention, prefix preview, Windows name diagnostics,
and the responsive Workbench are implemented and tested. Drag/drop,
virtualisation, plan JSON inspection, occupied-destination checks, and stale
source validation remain in Milestone 1.

The next implementation request should finish those read-only planning gaps. It
must not begin real rename execution until the complete plan format and recovery
design have been reviewed.

The expected first commit series is:

1. `feat: admit dropped sources without exposing native paths`
2. `feat: virtualise and filter large rename plans`
3. `feat: inspect and export versioned plan JSON`
4. `test: cover occupied destinations and stale source snapshots`

Commit boundaries may be combined only when the resulting change is genuinely
atomic and independently testable.
