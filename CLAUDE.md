# CLAUDE.md

This project is a fork of https://github.com/indygreg/linux-packaging-rs

It completes the implementation of debian-packaging/src/repository/s3.rs and
uses aws-sdk-s3 instead of rusoto_s3. Fixes some bugs and completes the
implementation so that any user can use `debian-packaging` as a complete
library crate.

## Syncing with upstream

Add the upstream remote once (if not already present):

```bash
git remote add upstream https://github.com/indygreg/linux-packaging-rs.git
```

Fetch upstream and rebase our changes on top of it:

```bash
git fetch upstream
git rebase upstream/main
```

Resolve any conflicts, then force-push the updated branch to origin:

```bash
git push --force-with-lease origin HEAD
```

## Building

This is a Cargo workspace with four crates: `debian-packaging`, `debian-repo-tool` (binary `drt`),
`linux-package-analyzer` (binary `lpa`), and `rpm-repository`. Minimum supported Rust version is **1.75**.

Build the entire workspace:

```bash
cargo build --workspace
```

Build without default features (verifies feature-gated code compiles cleanly):

```bash
cargo build --workspace --no-default-features
```

Lint:

```bash
cargo clippy --workspace
```

## Testing

Tests use [`cargo-nextest`](https://nexte.st/). Install it once:

```bash
cargo install --locked cargo-nextest
```

Run all tests:

```bash
cargo nextest run --workspace
```

Compile tests without running them (quick pre-flight check):

```bash
cargo nextest run --no-run --workspace
```

### Optional features in `debian-packaging`

Both features are enabled by default. To build or test without them:

| Feature | What it enables |
|---------|----------------|
| `http`  | HTTP repository fetching via `reqwest` |
| `s3`    | S3 repository backend (this fork's main addition) |

```bash
cargo nextest run -p debian-packaging --no-default-features
```

### Known pre-existing test failures

Two tests fail on this machine and are unrelated to this fork's changes:

- **`rpm-repository http::test::fedora_41`** — hits a live Fedora mirror URL
  that returns HTTP 404; the upstream URL has gone stale.
- **`debian-packaging control::tests::test_parse_system_lists`** — parses
  `/var/lib/apt/lists/` from the local system; fails on a malformed description
  field in the Microsoft VSCode package list.
