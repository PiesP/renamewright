<!-- Hallmark · pre-emit critique: P5 H5 E4 S5 R5 V4 -->

# Product design

## Product definition

Renamewright is a local-first rename workbench. Its primary job is not to
offer many isolated rename commands; it is to let a user compose a deterministic
rule pipeline, understand every proposed path, resolve risks, and then execute a
recoverable plan.

The product line is “Plan every rename.” The public application, repository, and
executable use `Renamewright`; machine-facing identifiers use `renamewright`.
The durable activity, recovery, and undo surface is named **Rename Ledger**. It
is a feature within Renamewright, not a separate product or financial metaphor.

The working audience is people who rename tens to thousands of files on Windows,
from occasional users organising downloads to advanced users preparing media,
archives, and project assets. The first release is Windows-first. The domain
model remains portable so Linux and macOS support can follow without weakening
Windows filename validation.

The visual tone is technical and utilitarian. Hallmark's `Workbench`
macrostructure and modern-minimal Cobalt family guide the interface: cool neutral
surfaces, one restrained signal colour, compact controls, strong keyboard focus,
and status carried by hierarchy rather than decorative cards or animation.

## Product principles

1. **Preview is the product.** Every rule updates an immutable proposal before
   any filesystem mutation is possible.
2. **Safe by construction.** The executor receives a validated plan, never a bag
   of arbitrary frontend paths and replacement strings.
3. **Explain every refusal.** Errors identify the affected entry, the violated
   rule, and the next useful action.
4. **Make order visible.** Rules are a pipeline. Reordering is deliberate,
   keyboard accessible, and immediately reflected in the preview.
5. **Undo is evidence-based.** The app records what happened and revalidates the
   current filesystem before offering a reversal.
6. **Paths are not text.** The Rust backend retains native `PathBuf`/`OsString`
   values and gives the UI stable IDs plus display projections.
7. **Local means local.** No telemetry, remote UI, account, cloud sync, or
   automatic upload is required to rename files.

## Primary workflow

1. Add files through the native picker or drag and drop.
2. Add and reorder rules in the rule rail.
3. Review the live preview table and filter to changed, warning, or blocked rows.
4. Open the review drawer to see collisions, invalid names, stale metadata, and
   the exact execution scope.
5. Apply the plan. Progress is streamed from the Rust executor.
6. Review the Rename Ledger or request an undo after revalidation.

The Apply action stays unavailable while any blocking diagnostic exists. A
confirmation modal is not the main safety mechanism; the persistent review
surface is. Destructive-looking surprises are solved in the plan, not hidden
behind an extra click.

## Interface structure

The desktop window uses four persistent regions instead of a dashboard of equal
cards:

- **Source bar:** add files, reveal scan scope, save/load presets, and expose the
  current session name.
- **Rule rail:** an ordered list of transformations with enable, edit, duplicate,
  reorder, and remove actions. One selected rule opens its inline editor.
- **Preview table:** original name, proposed name, parent, status, and selected
  metadata. It supports virtualization, column visibility, sorting, selection,
  and per-item overrides without changing rule order.
- **Review bar and drawer:** changed/unchanged/blocked counts, diagnostics, Apply,
  cancellation state, Rename Ledger, and Undo.

At narrower window widths, the rule rail becomes a resizable overlay and the
review drawer occupies the full content region. The preview never silently hides
blocking status. A minimum supported window size will be established by the
first rendered prototype rather than guessed in this document.

## Rule model

Rules are explicit, ordered, serializable, and versioned. The initial product
surface groups legacy one-off commands into a smaller coherent vocabulary:

- literal or linear-time regular-expression replacement;
- prefix, suffix, and token template insertion;
- sequence numbering with scope, start, step, width, and direction;
- case conversion and whitespace/punctuation cleanup;
- Unicode normalization as an explicit opt-in rule;
- extension preserve, remove, replace, or normalise;
- substring/range extraction and removal;
- keep/remove character classes, including digits;
- per-entry override after the shared pipeline.

Template tokens may include stem, extension, parent-folder name, sequence, and
selected filesystem timestamps. Content metadata such as EXIF, ID3, and media
duration is deferred until the core transaction model is proven.

Every rule editor must have default, hover, focus, active, disabled, loading,
error, and success treatment where the state applies. Motion is limited to
opacity/transform transitions that communicate reordering or panel state, with a
reduced-motion path.

## Diagnostics

Diagnostics have three levels:

- **Blocked:** applying would overwrite an unrelated entry, create an invalid
  component, lose source identity, cross an unsupported boundary, or execute an
  internally inconsistent plan.
- **Warning:** the result is legal but deserves attention, such as hidden files,
  leading/trailing normalisation, symlink entries, or a very large batch.
- **Information:** unchanged rows and rules that produced no effect.

Rows show a concise reason. The review drawer groups related diagnostics and can
focus the corresponding row or rule. Colour never carries severity alone.

## Deliberate changes from the legacy application

- Separate commands become a visible, reusable rule pipeline.
- Sorting affects review order only; numbering scope is explicit and does not
  depend on an invisible current list order.
- Two-phase temporary renames handle swaps, cycles, and case-only changes.
- A durable activity journal replaces an in-memory notion of “original name.”
- Text-list import/export becomes a versioned preset and plan format; CSV export
  remains available for inspection but is not executable input by default.
- Direct editing becomes a per-entry override that remains visible after rules
  change.
- Broad folder/path rewriting is replaced with constrained filename templates in
  v1. Moving entries between directories is a separate future feature.

## Initial non-goals

- Renaming directories or moving entries between parent directories.
- Modifying file contents or embedded metadata.
- Following directory symlinks during discovery.
- Cloud presets, accounts, collaboration, telemetry, or an auto-updater.
- Shell commands, user-provided scripts, or plugin execution.
- Exact layout, timing, sorting, or failure compatibility with the old EXE.

## Product acceptance baseline

The first releasable version must demonstrate all of the following on Windows:

- A user can add at least 10,000 files without the UI becoming unusable.
- Preview output is deterministic for the same source snapshot and rule set.
- Every blocked plan explains why Apply is unavailable.
- Swaps, cycles, and case-only renames complete through temporary names.
- An injected failure triggers a tested best-effort rollback and leaves an
  inspectable recovery journal.
- Undo refuses stale or externally changed entries instead of overwriting them.
- Keyboard-only operation covers source admission, rule editing/reordering,
  preview review, Apply, cancellation, and recovery.
- Korean and English strings come from message catalogs; filenames are never
  round-tripped through a lossy display string.
