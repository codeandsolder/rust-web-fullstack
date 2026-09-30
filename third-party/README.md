# Temporary compatibility patches

These crates are exact copies of the crates.io releases selected by `Cargo.lock`,
with narrowly scoped source-compatible fixes for Rust future-incompatibility
warnings. They are wired through the root `[patch.crates-io]` table and are
excluded from workspace membership.

## manyhow 0.11.4

Upstream crate: `manyhow 0.11.4`.

Patch: remove the trailing semicolon from each `bail!` expansion. The macro
still expands to the same early `return Err(...)`, but callers can use it in
expression position without triggering Rust issue #79813
(`semicolon_in_expressions_from_macros`).

This removes the future-incompatibility warnings currently attributed to
`attribute-derive-macro 0.10.5`.

## proc-macro-error2 2.0.1

Upstream crate: `proc-macro-error2 2.0.1`.

Patch: make the crate's `extern crate proc_macro` public because its hidden
`__export` module publicly re-exports `proc_macro`. This is the compiler's
suggested source-compatible fix for Rust issue #127909 / E0365.

`proc-macro-error2` is unmaintained; prefer removing this patch when the
upstream dependency chain migrates to a maintained replacement.
