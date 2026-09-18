# `cargo-msrv-pin`

A `cargo` subcommand that downgrades your `Cargo.lock` dependencies to the
highest versions still compatible with a given Rust toolchain.

```bash
$ cargo update
    Updating crates.io index
     Locking 20 packages to latest compatible versions
    Updating pest v2.8.0 -> v2.8.2
    Updating pest_derive v2.8.0 -> v2.8.2
    ...
$ cargo msrv-pin 1.75.0
INFO: 2 incompatible package(s) remaining
INFO: updated pest_derive 2.8.2 -> 2.8.0
INFO: 1 incompatible package(s) remaining
INFO: updated pest 2.8.2 -> 2.8.0
INFO: verifying build...
INFO: done
```

## Why

A project's own MSRV (minimum supported Rust version) is only half the
picture: `cargo update` happily locks dependencies to versions whose *own*
MSRV has moved past your toolchain, breaking the build with no obvious
connection to what was just updated. `cargo-msrv-pin` finds every such
dependency and pins it back down, one package at a time, until the whole
dependency graph builds again on the target toolchain.

This is a routine problem when cross-compiling with Yocto/OpenEmbedded: the
host's Rust, kept current through `rustup`, is always newer than the one
bundled in a given Yocto release, which stays fixed to whatever it shipped
with. `cargo update` on the host happily picks dependency versions the host
toolchain builds fine, and the next `bitbake` build then fails deep inside
some dependency's `Cargo.toml` with no obvious link to anything the project
itself changed.

## Install

```bash
cargo install cargo-msrv-pin
```

Or, from a local checkout:

```bash
cargo install --path .
```

## Usage

```bash
cargo msrv-pin <toolchain> [-C <path>] [-v]
```

- `<toolchain>`: the target toolchain, e.g. `1.75` or `1.75.0`.
- `-C <path>`: run as if `cargo-msrv-pin` had been started in `<path>`.
- `-v`, `--verbose`: show debug logs (each command run, each skip reason).

> [!NOTE]
> `cargo-msrv-pin` runs several `cargo +<toolchain>` commands (including the
> final `cargo +<toolchain> check` verification step), so the target
> toolchain needs to be available:
>
> - By default, rustup auto-installs a missing toolchain the first time
>   it's referenced with `+<toolchain>`. Nothing to do.
> - If that's disabled, install it yourself first:
>   ```sh
>   rustup toolchain install 1.75.0
>   ```

## How it works

1. If `Cargo.lock` was written by a newer `cargo` than the target toolchain
   understands, delete it and let the target toolchain regenerate it first.
1. Read `cargo metadata` to find every registry dependency whose declared
   `rust_version` exceeds the target toolchain.
1. For each one, query crates.io for the highest non-yanked, non-pre-release
   version whose own `rust_version` fits the target toolchain.
   - If none exists, the package might still only be in the graph because
     something depending on it chose to pull in that version, not because
     of a real constraint (e.g. package A depends on package B only for a
     certain target or feature). Try downgrading that dependent instead,
     climbing further up the graph if needed.
   - If it's a direct dependency whose own `Cargo.toml` entry rules out
     that version, relax the entry first. Otherwise the downgrade would
     fail the same way no matter how many times it's retried.
1. Downgrade one package at a time with `cargo update --precise`, re-reading
   `cargo metadata` after every update. A single `cargo update` can also
   move unrelated transitive dependencies, so the whole picture is
   re-evaluated each time rather than assumed.
1. A package is left for a later pass when another still-incompatible
   package holds a version constraint on it (e.g. package A requires a
   newer version of package B, so B can't be downgraded until A is
   downgraded first).
1. Once nothing is left to downgrade, run `cargo +<toolchain> check` to
   confirm the build actually succeeds. Packages that couldn't be
   downgraded (e.g. platform-specific dependencies not compiled for the
   current target) are only a problem if the build says so.

## License

Released under the [MIT License](LICENSE.md).
