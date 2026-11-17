# Compiler diagnostics

Better error messages, continuing after initial errors, etc.

| ID     | Name                            | Owner       | Status       |
|--------|---------------------------------|-------------|--------------|
| DIAG-1 | Structured error types          | @LesterEvSe | Proposed     |
| DIAG-2 | Continue after any failed item  | @LesterEvSe | Proposed     |
| DIAG-3 | Identifier spans                | @LesterEvSe | Needs design |
| DIAG-4 | Analyze code with syntax errors | @LesterEvSe | Needs design |
| DIAG-5 | Cost report                     | @LesterEvSe | Needs design |

## Goal

`simc` should report all real errors of a program in one run, and only the real ones.
Fixing one error should remove exactly one diagnostic: an error that exists only because of an earlier one must not be
reported.

## Done

Tracked in #362.

| Change                                                         | PR   |
|----------------------------------------------------------------|------|
| The parser recovers and reports several syntax errors per file | #185 |
| Every span carries its `file_id`                               | #363 |
| Errors are collected as values instead of `String`s            | #369 |
| `Diagnostic`, rendering with `ariadne`                         | #370 |
| The `!` type                                                   | #417 |
| Analysis reports through `DiagnosticManager`                   | #418 |
| Every error inside an expression is reported                   | #420 |
| Analysis continues after a broken function body                | #424 |
| Analysis continues after an undefined type                     | #430 |
| Analysis continues after a failed statement                    | #448 |

On master, analysis still stops in two cases:

- A failed item other than `main`, such as a broken `use` or a redefined alias, stops the analysis of the remaining
  items (DIAG-2).
- A syntax error in any file stops the compilation before analysis (DIAG-4).

## DIAG-1: Structured error types

One error type per phase.
No `String` errors in the public API.

### Design

Today all 66 variants, from dependency resolution to witness checking, live in one `error::Error` enum.
Some of them, like `Grammar { msg }` and `CannotParse { msg }`, are just a free-form message used for many unrelated
errors.
For example, an enum without variants and an enum with a duplicate variant name are both `Grammar`.
Some public functions also still return `Result<_, String>`: `TemplateAst::instantiate`, `CompiledProgram::satisfy`,
`CompiledProgram::satisfy_with_env`.
For a caller, the only way to tell these errors apart is to parse the message.

Each phase gets its own error enum with `thiserror`, next to the code that creates it:

| Enum              | Phase                             |
|-------------------|-----------------------------------|
| `ResolutionError` | dependency map, file loading      |
| `VersionError`    | `simc` version directive          |
| `ParseError`      | lexer, parser                     |
| `ScopeError`      | name resolution                   |
| `TypeError`       | analysis of items and expressions |
| `WitnessError`    | arguments and witness values      |

`error::Error` keeps one variant per phase, so `Diagnostic` and `DiagnosticManager` stay as they are.

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Scope(#[from] ScopeError),
    // ...
}
```

The new types follow apoelstra's guidelines from #324:

- No `String` errors, unless the wrapped error is already a `String`.
- No messages like `cannot parse: {0}`.
  The wrapped error is available through `source()`, and the renderer prints the `source()` chain.
- Named fields: `FileNotFound { path: PathBuf }`, not `FileNotFound(String)`.
- No `From` impl that only wraps another error if the call site could add more context.

While converting, `Grammar` and `CannotParse` get split into separate variants, the functions above get typed errors,
and the variants that are never constructed go away.

Tests compare error variants instead of rendered messages.
Today `assert_errors` in `src/test_utils.rs` compares `to_string()` output, so rewording a message breaks tests that
check the error kind, while `src/error_cases.rs` already matches on variant names.
A few tests per phase keep checking the rendered text, since that is what users read.

The work starts from gerau's draft in #324.

## DIAG-2: Continue after any failed item

A broken item no longer stops the analysis of the items after it.

### Design

`analyze_items` keeps going after a failed `main`, but returns at the first other item that fails.
Nothing after that item gets analyzed:

```rust
mod math {
    pub fn double(a: u32) -> u32 { a }
}

use crate::math::{double, triple}; // reported: `triple` could not be found

fn main() {
    let d: bool = double(1); // never reported
}
```

Broken function bodies (#424) and undefined types (#430) don't stop it anymore.
What still does: a failed `use`, a function, alias, enum or module defined twice, an enum without variants or with a
repeated variant, and any of these inside a `mod`, because then the whole module fails.

A failed item should still bind its names, and every use of those names should stay silent.
Then the items after it see the same names as on success, and nothing else gets reported because of it.

- A failed `use` binds each imported name in every namespace where the name is still free, since we can't know if it
  was a function, a type or a module. #451 does this.
- A redefinition is reported, and the name keeps its first definition.
  The second one is still analyzed, so errors inside it show up too, as they already do for functions.
- An enum without variants is bound as `!`, which is what an empty enum is anyway.
  An enum with a repeated variant keeps the first one.
- A module is always bound.
  If it's defined twice, the second one is analyzed on its own and paths go to the first.

After fixing these and removing the early return from `analyze_items`, #207 can be closed.

MOD-1 (#458) replaces the resolver.
If it lands first, we do the rest of DIAG-2 in the new resolver, and skip whatever MOD-1 already fixes.
Either way, the tests in `src/ast/tests/multi_error.rs` should keep passing.

### `use` and namespaces

A `use` looks its name up in three namespaces (types, functions, modules), and `resolve_processing_use_items_error`
keeps only one result.
If one namespace succeeds, the errors from the others are dropped.
For example, this compiles, although the local `type f` clashes with the imported one:

```rust
mod m {
    pub fn f() -> u8 { 0 }
    pub type f = u8;
}

type f = u16;
use crate::m::f; // the function is imported, the type clash is ignored
```

When a name is missing everywhere, the error doesn't say where it looked or what it found (#348).

Propose fixes:

| In the target module       | Today                                             | With DIAG-2                      |
|----------------------------|---------------------------------------------------|----------------------------------|
| public                     | nothing                                           | nothing                          |
| public, name already taken | `RedefinedItem`, unless another namespace imports | `RedefinedItem`                  |
| private only               | `PrivateItem` without the item kind               | `PrivateItem` with the item kind |
| not found                  | `UnresolvedItem` without the module               | `UnresolvedItem` with the module |
