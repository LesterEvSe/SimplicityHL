# Compiler diagnostics

Better error messages, continuing after initial errors, etc.

| ID     | Name                            | Owner       | Status       |
|--------|---------------------------------|-------------|--------------|
| DIAG-1 | Structured error types          | @LesterEvSe | Proposed     |
| DIAG-2 | Continue after any failed item  | @LesterEvSe | Proposed     |
| DIAG-3 | Identifier spans                | @LesterEvSe | Proposed     |
| DIAG-4 | Analyze code with syntax errors | @LesterEvSe | Proposed     |
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

## DIAG-3: Identifier spans

Diagnostics point at the name, not at the whole item.

### Design

#### Today

The parser reads names into the string types of `src/str.rs` and drops their spans.
Only the node around a name has a span, so an error about the name points at the whole node:

```rust
fn add(a: u32, b: u32) -> u32 {
    a
}

fn add(a: u32, b: u32) -> u32 {
    b
}
```

```text
Error: Function `add` was defined multiple times
   ╭─[main.simf:5:1]
   │
   ┆
───╯
```

The label covers all of lines 5 to 7.
`simc` prints no source line for it, and the editor underlines the whole function.
An undefined type in a signature, such as `fn total(a: u64) -> Amount`, is reported the same way.

The LSP needs these spans too (#178), because it currently finds them by lexing the source again: see
`find_function_name_range`, `find_imported_function` and `get_call_span` in [simplicityhl-lsp#c48b265](https://github.com/BlockstreamResearch/simplicityhl-lsp/blob/c48b265e342358e2da5dfcc07c3259f5addff9d7/src/navigation/mod.rs).

#### Proposed spans

We propose to store a span next to each name in the parse tree, and on every node of a type:

| Parse node                     | Span today                  | Proposed                                       |
|--------------------------------|-----------------------------|------------------------------------------------|
| `Function`                     | the whole function          | also the name                                  |
| `FunctionParam`                | `a: u8`                     | also the name                                  |
| `TypeAlias`, `EnumDeclaration` | the whole item              | also the name                                  |
| `EnumVariant`                  | the whole variant           | also the name                                  |
| `Module`                       | the whole module            | also the name                                  |
| `UseDecl`                      | the whole `use`             | each path segment, imported name and `as` name |
| `Call`                         | the call with its arguments | the callee, and `f` in `fold::<f, N>`          |
| `EnumMatchArm`                 | the whole arm               | also the variant name                          |
| `EnumConstruction`             | the whole expression        | also the variant name                          |
| `AliasedType`                  | none                        | every node, e.g. `Amount` in `(u8, Amount)`    |

Literals and variables in expressions already have a span, since the expression is the literal or the variable.

The spans must live in the parse nodes, not inside the `src/str.rs` types.
Parse nodes already leave spans out of equality and hashing (`impl_eq_hash!`), so nothing changes in how nodes compare.

Types are different. `AliasedType` is the type as written: analysis turns it into `ResolvedType`.
So every node of `AliasedType` gets its own span, which equality and hashing ignore.
An undefined alias inside a larger type, such as `Amount` in `(u8, Amount)`, then points at `Amount` only, as in Rust.
Types built in code, like jet signatures in `src/jet` and types read from `.wit` files, have no span.

A name in the parse tree is wrapped together with its span, so its span is read like that of any other parse node:
`function.name().span()`, and the same for each item of a `use` list.

The type spans also let the LSP show what a type is on hover, like what `Pubkey` stands for or what `u8` is.

#### What each error points at

| Error                           | Label today                                | Label with DIAG-3              |
|---------------------------------|--------------------------------------------|--------------------------------|
| `FunctionRedefined`             | the whole second function                  | the second name                |
| `RedefinedAlias`                | the whole second alias                     | the second name                |
| `ParameterRedefined`            | the whole second parameter                 | the second name                |
| `ModuleRedefined`               | the whole first module                     | the second name, as in MOD-1   |
| `UnresolvedItem`, `PrivateItem` | the whole `use`                            | the item in the `use` list     |
| `RedefinedItem`                 | the whole `use`                            | the item in the `use` list     |
| `UndefinedAlias`                | the enclosing function, `let`, alias, call | the undefined name             |
| `FunctionUndefined`             | the call with its arguments                | the function name              |
| `MainCannotBePublic`            | the whole `main`                           | the name `main`                |
| `MainNoInputs`                  | the whole `main`                           | the parameters                 |
| `MainNoOutput`                  | the whole `main`                           | the return type                |

Errors about a variable or a witness in an expression already point at the name, since the expression is just the name.

#### Steps

- Names of items and parameters.
Redefinitions, `MainCannotBePublic` and `MainNoInputs` point at the name.
  The LSP can drop `find_function_name_range`.
- Types. `UndefinedAlias` points at the undefined name and `MainNoOutput` at the return type, so no error covers a whole function any more.
  The LSP can show hover information for types.
- `use` paths and items. `use` errors point at the item. (Also influenced by MOD-1)
  The LSP can drop `find_imported_function`.
- Call names. `FunctionUndefined` points at the name.
  The LSP can drop `get_call_span` and the lexing in semantic tokens.

### Out of scope

- Pointing a redefinition also at the first definition, as a secondary label (`Diagnostic::with_secondary`).
- Variables in patterns, such as `a` in `let (a, b) = ...`.
- The rest of what the LSP asks for in #191. Currently the LSP has its own resolver, and parses a file without `main` by appending `fn main() {}` to it. It should be a separate roadmap item.
- Jet definitions, as follow-up work in the LSP.
Jets have no source file: their signatures are built in `src/jet` and implemented in C in libsimplicity.
With the span of the jet name at the call, the LSP can show its docs on hover, or open a generated read-only file with all jet signatures, docs and costs, as `codegen` already does for Rust.

## DIAG-4: Analyze code with syntax errors

Parser error nodes. Files with syntax errors are analyzed.

### Design

#### Today

The parser recovers from syntax errors and reports several of them per file (#185), but analysis never sees the
recovered tree.
`parse_from_str_with_errors` returns no tree for a file with any error (see the TODO in `src/parse/pipeline.rs`), and
the driver builds no program if any file had an error (`if diagnostics.has_errors()` in `src/driver/mod.rs`).
So one syntax error hides every other error:

```rust
fn double(a: u32) -> u32 {
    let b: u32 = a // reported: Expected `;` after statement
    b
}

fn main() {
    let c: bool = double(1); // never reported
}
```

The LSP gets no tree for a file with a syntax error, so go-to-definition, document symbols and semantic tokens stop
working in it.
While the user types, the file almost always has one.

Instead of made-up values, the parser should leave error nodes where it recovered, analysis gives them the type `!` without reporting.
So we suggest this kind of structure to analyze files with syntax errors:

| The parser can't read       | Today                         | Proposed                                         |
|-----------------------------|-------------------------------|--------------------------------------------------|
| An item                     | `Item::Ignored`               | the item, if its name was read, with error nodes |
| A function body or a block  | an empty block, `()`          | an error expression                              |
| A parameter list            | an empty list                 | error parameters                                 |
| A `match`                   | a match with placeholder arms | an error expression                              |
| A module body               | a module without items        | a broken mark, with silent paths into it         |
| Generic arguments of a type | an alias named `error`        | an error type                                    |
| A tuple type                | `()`                          | an error type                                    |
| A length in a type          | `0`, as in `[u8; 0]`          | an error type                                    |

#### Steps

Steps 1 to 3 don't depend on each other and can land in any order.
Step 4 comes last.

1. Error expressions, error types and error parameters, in place of the empty block, the placeholder `match`, the empty parameter list, the `error` alias, `()` and `0`. Analysis types them `!` without reporting.
2. The broken mark on modules, with silent paths into them.
A `use` into a broken module binds its names silently, as a failed `use` does in DIAG-2.
3. Items keep their name once it's read, and the broken parts get error nodes.
4. `parse_from_str_with_errors` returns the tree with its errors, and the driver analyzes all files. Needs DIAG-2, since broken items must not stop the analysis.

A file with a bad `simc` directive is not analyzed at all, because errors under another version can be incorrect.
