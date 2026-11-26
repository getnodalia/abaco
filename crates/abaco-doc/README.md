# abaco-doc

The Ábaco maths document. An ordered list of **math** and **text** cells; math cells are
written in the `abaco-expr` language. The document works out which cell defines which name,
evaluates cells in dependency order (not page order), and re-evaluates only what an edit
affects. Documents are saved as `.abaco` JSON files.

## Public API

`Document` holds the cells (`push_math`, `push_text`, `insert`, `set_source`, `set_text`,
`remove`, `move_cell`, `cells`, `cell`, `position`), the `Settings` (`significant_digits`,
`strict_order`) and the evaluation `Limits`. Edits only mark work; `evaluate()` does it and
returns the cells it ran, in order. `result(id)` gives a `CellResult`: `Ok(CellOutput { value,
defined })` or a `CellError`, plus warnings. `report()` is a JSON account of every cell
(`status` `ok`, `error`, `text` or `pending`; error `code`, `message`, 1-based `line` and
`column` inside the cell). `to_json`/`from_json`/`open`/`save` read and write the file format.
`evaluations()` and `last_evaluated()` expose how much work was done. A `CellId` is stable
while the cell exists and is never reused.

## Rules of evaluation

* A cell **defines** the names it assigns (`x = ...`, `f(x) = ...`) and **uses** the names it
  reads that it did not define earlier in the same cell. A cell depends on the cells that define
  the names it uses. Names defined by no cell are constants, units and built-ins, as in
  `abaco-expr`. A variable defined by a cell therefore shadows a unit or constant of the same
  name in every cell that uses it.
* A name may be defined by one cell. The first cell on the page owns it; later cells that
  define it get a `Duplicate` error.
* A circular definition gives every cell of the cycle a `Cycle` error whose message shows the
  path (``circular reference: `a` -> `b` -> `a` ``); a cell that only uses a cycle gets a
  `Dependency` error.
* One failing cell never blocks unrelated cells. A cell that uses a failed cell gets a
  `Dependency` error naming it. Syntax errors keep their position inside the cell.
* With `strict_order = true` a cell that uses a name defined further down the page gets a
  warning (it is still evaluated, because order follows dependencies).
* Cells see only the names they use: other cells' values are copied in, and functions are
  closures over the names visible where they were defined.
* **Incremental**: a cell runs again when it is new or edited, when it resolves a name
  differently than before (a definition was added, removed or moved), or when a cell it uses
  produced a different outcome. A cell whose outcome comes out identical does not wake its
  dependents. The test suite counts evaluations to check this, and checks (property tests of
  random edit sequences) that an incrementally maintained document always equals the same
  document evaluated from scratch.
* Display precision is a setting, applied when values are shown; changing it does not
  re-evaluate anything.

## The `.abaco` format (version 1)

JSON with the keys `format` (`"abaco"`), `format_version` (1), `settings`, `next_cell_id`,
`cells` (each `{"type": "math", "id", "source"}` or `{"type": "text", "id", "markdown"}`) and
`engine` (versions that wrote the file; informational, ignored on read). Keys and cells are
written in a fixed order, so saving twice gives identical bytes. Results are not stored.

On read: a file larger than 16 MB, with more than 20,000 cells, a math source over 100,000
bytes, a text cell over 1,000,000 bytes, an id outside 1 to 2^53-1, a repeated id, or settings
out of range is refused with a message. A `format_version` above 1 is refused as a newer file
(checked before anything else is read, so new cell types in it do not matter). **Unknown
fields in a version-1 file are ignored and are not preserved on save.** A low `next_cell_id` is
raised above the largest id so ids are never reused. Errors in malformed JSON carry the line
and column. `save` writes beside the target and renames, so a crash never leaves half a file;
`open` checks the size before reading.

## Limits and honest notes

Each cell is bounded by the expression limits (steps, elements, depth); a document-wide time
budget does not exist, so a document of many maximum-cost cells can still take long. A 5,000-cell
document (a 2,500-link dependency chain plus 2,500 independent unit conversions) evaluates in
about 80 ms in an unoptimised test build on the development machine, and editing the head of
the chain re-evaluates its 2,500 dependents in about 25 ms. Graph analysis is iterative, so a
cycle through thousands of cells does not overflow a small thread stack. No plot cell exists
yet. The `engine` block lists `abaco-doc`, `abaco-expr` and `nodalia-units` versions;
`nodalia-numerics` does not expose its version yet.

Run `cargo test -p abaco-doc`.
