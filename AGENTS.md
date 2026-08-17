# AGENTS.md

Operational guidance for coding agents working in this repository.

## Project Purpose

`chnroute` is a Rust command-line program that fetches and generates IPv4 and
IPv6 route tables by region and network operator.

The current implementation provides only the `fetch` command. A `generate`
command is planned but must remain a separate, offline stage.

## Data Model

The upstream operator source is:

```text
https://github.com/gaoyifan/china-operator-ip
```

Trust upstream membership for these sets. Do not reclassify them by applying
local geolocation or operator heuristics:

- Mainland China: `china`
- China Telecom: `chinanet`
- China Mobile: `cmcc`
- China Unicom: `unicom`
- CERNET: `cernet`
- CSTNET: `cstnet`
- Dr.Peng: `drpeng`
- Google China: `googlecn`

The planned generated sets have these semantics:

- `private`: built-in project knowledge.
- `special`: built-in special-use and other non-public address knowledge,
  excluding `private`.
- `china`: the upstream `china` set.
- `nonchina`: the public address space minus `china`.
- `chinanet`: the upstream `chinanet` set.
- `cmcc`: the upstream `cmcc` set.
- `unicom`: the upstream `unicom` set.
- `china-other`: the union of `cernet`, `cstnet`, `drpeng`, and `googlecn`.

Operator sets may overlap. Do not invent an operator precedence rule. Set
normalization may change CIDR representation, but it must not change set
membership.

## Fetch Contract

`chnroute fetch` is an input acquisition command only. Keep these properties:

- Resolve a branch or tag to one full commit SHA before downloading files.
- Use Git Smart HTTP reference discovery, not the GitHub REST API. This avoids
  unauthenticated API rate limits.
- Download all 16 fixed IPv4 and IPv6 source files from the same commit.
- Preserve downloaded file bytes exactly. Do not sort, merge, or rewrite raw
  files.
- Validate every non-empty line as a canonical CIDR of the expected address
  family.
- Accept an empty upstream file as a valid empty set.
- Record source identity, hashes, sizes, address families, and prefix counts in
  `manifest.json`.
- Publish a complete snapshot through a staging directory. A failed download
  or validation must leave the previous snapshot intact.
- Refuse to replace a symlink or a non-empty directory not owned through a
  compatible `manifest.json`.
- Keep repeated fetches idempotent unless `--force` is supplied.

Tests must use the in-memory fake HTTP client. Unit and integration tests must
not depend on live network access.

## CLI Output Contract

All command-line output must be English ASCII. This includes:

- Top-level and subcommand help.
- Argument descriptions.
- Success, no-op, warning, and error messages.
- Clap parser errors.
- Dynamic values copied into diagnostics.

Keep user-facing strings in `src/` ASCII. Route all dynamic terminal text
through the top-level `ascii_only` boundary so non-ASCII input is rendered as
ASCII `\u{...}` escapes. Do not call Clap's direct exit or print helpers because
they bypass that boundary. Keep terminal color disabled for deterministic
output.

The Chinese README is not subject to the CLI ASCII rule. `AGENTS.md` itself
must remain English ASCII.

## Repository Layout

```text
Cargo.toml       Rust package and dependencies
Cargo.lock       Locked dependency graph
src/main.rs      CLI parsing and terminal output boundary
src/fetch.rs     Fetch workflow, validation, manifest, and tests
src/lib.rs       Library module exports
data/raw/        Downloaded upstream snapshot and manifest
README.md        User documentation
```

Place future IP set algebra in a dedicated module such as `src/ipset.rs`.
Place future generation orchestration in `src/generate.rs`. Do not mix network
access into generation code.

## Rust Conventions

- Use stable Rust and edition 2024.
- Keep production code free of `unwrap` and `expect` when input, I/O, or
  network state can fail. They are acceptable in focused tests.
- Add context to I/O and network errors.
- Prefer deterministic data structures and output ordering.
- Keep raw acquisition, set algebra, generation policy, and terminal output as
  separate concerns.
- Preserve the blocking HTTP implementation unless measured requirements
  justify adding an async runtime.
- Use `apply_patch` for intentional source edits. Do not overwrite unrelated
  or untracked user work.

## Required Checks

Run these checks after Rust changes:

```text
cargo fmt --all -- --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
```

Also verify that Rust source contains no non-ASCII text:

```text
rg -nP '[^\x00-\x7F]' src
```

For changes to `fetch`, perform a live upstream smoke test only when network
access is authorized. Never make the normal test suite require network access.
