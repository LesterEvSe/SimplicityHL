# Compiler diagnostics

Better error messages, continuing after initial errors, etc.

| ID     | Name                            | Owner       | Status       |
|--------|---------------------------------|-------------|--------------|
| DIAG-1 | Structured error types          | @LesterEvSe | Proposed     |
| DIAG-2 | Continue after any failed item  | @LesterEvSe | Needs design |
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
