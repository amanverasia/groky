<div align="center">

<h1>groky</h1>

**groky** is a community-maintained fork of [xAI's Grok CLI](https://github.com/xai-org/grok-cli)
(Grok Build) — a terminal-based AI coding agent. It runs as a full-screen TUI
that understands your codebase, edits files, executes shell commands, searches
the web, and manages long-running tasks — interactively, headlessly for
scripting/CI, or embedded in editors via the Agent Client Protocol (ACP).

**[groky.dev](https://groky.dev)** — website, documentation, and installer

[What's different](#whats-different-from-grok) ·
[Installation](#installation) ·
[Documentation](#documentation) ·
[Repository layout](#repository-layout) ·
[Development](#development) ·
[Roadmap / TODO](#roadmap--todo) ·
[License](#license)

</div>

---

## What's different from Grok

groky keeps everything the upstream CLI does and adds:

- **No networked telemetry.** Sentry, Mixpanel, OTLP export, and trace upload
  are removed entirely (~20k lines). Diagnostics stay local (log files under
  `~/.grok/`), and a scan gate (`bin/check-no-network-telemetry.sh`) guards
  against reintroduction.
- **Multi-provider model catalog.** A bundled [models.dev](https://models.dev)
  snapshot ships 150+ providers and 4,500+ models (OpenAI, Anthropic,
  OpenRouter, Groq, Mistral, Together, ...) with a 24-hour background refresh
  and offline-safe caching.
- **`/providers` key management** (alias `/login`). Add API keys per provider
  through a masked prompt; keys are stored in `~/.grok/auth.json` with `0600`
  permissions. A credential firewall guarantees xAI tokens never reach
  third-party providers.
- **Provider-aware model picker.** `/model` (Ctrl+M) fuzzy-searches across
  every configured provider with weighted ranking, and preserves your current
  selection across catalog refreshes.
- **Dynamic OpenAI-compatible providers.** Bounded `GET /models` discovery
  with a strict URL security policy (HTTPS required, plain HTTP loopback-only
  unless explicitly confirmed, credential-stripping redirects, 2 MiB / 2,000
  model response caps) and last-known-good model caching.
- **First-class [Janus](https://groky.dev) support.** Guided setup in
  `/providers`: local gateway preset (`http://127.0.0.1:20128/v1`), health
  check, model discovery, optional `sk-janus-` key — with exact model ID
  preservation (`openai/gpt-4o`, `best-effort`, ...).

See [`docs/configuration/providers.md`](docs/configuration/providers.md) for
the full provider configuration guide.

## Installation

### One-line installer

Installs the latest release to `~/.local/bin/groky` (Linux,
x86_64 and aarch64; no sudo):

```sh
curl -fsSL https://raw.githubusercontent.com/amanverasia/groky/main/install.sh | bash
```

Options via environment variables: `GROKY_VERSION=v0.1.0` pins a release,
`GROKY_INSTALL_DIR` overrides the install directory. The script verifies the
sha256 checksum of every download. Once groky.dev is live, the same installer
will be served from `https://groky.dev/install.sh`.

Prebuilt binaries and checksums are also on the
[releases page](https://github.com/amanverasia/groky/releases).

### Building from source

Requirements:

- **Rust** — the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml);
  [`rustup`](https://rustup.rs) installs it automatically on first build.
- **protoc** — proto codegen resolves [`bin/protoc`](bin/protoc) (a
  [dotslash](https://dotslash-cli.com) launcher) or falls back to a `protoc`
  on `PATH` / `$PROTOC`.
- **Build host** — macOS and Linux are supported; Windows builds are
  best-effort and not currently tested from this tree.

```sh
git clone https://github.com/amanverasia/groky.git
cd groky
cargo run -p xai-grok-pager-bin              # build + launch the TUI
cargo build -p xai-grok-pager-bin --release  # release binary: target/release/groky
cargo check -p xai-grok-pager-bin            # fast validation
```

The binary artifact is named `groky` (upstream ships it as `grok`).
Configuration lives under `~/.groky` (override with `GROKY_HOME`; legacy
`GROK_HOME` and an existing `~/.grok` are honored — the latter is migrated
automatically on first launch). Add your provider API keys with `/providers`
on first launch, or sign in with an xAI account.

## Documentation

Documentation will live at **[groky.dev](https://groky.dev)** (not live yet —
see [Roadmap / TODO](#roadmap--todo)).

Until then, the user guide ships in-tree with the pager crate:
[`crates/codegen/xai-grok-pager/docs/user-guide/`](crates/codegen/xai-grok-pager/docs/user-guide/)
— getting started, keyboard shortcuts, slash commands, configuration, theming,
MCP servers, skills, plugins, hooks, headless mode, sandboxing, and more.
Provider configuration is documented in
[`docs/configuration/providers.md`](docs/configuration/providers.md).

## Repository layout

| Path | Contents |
|------|----------|
| `crates/codegen/xai-grok-pager-bin` | Composition-root package; builds the `groky` binary |
| `crates/codegen/xai-grok-pager` | The TUI: scrollback, prompt, modals, rendering |
| `crates/codegen/xai-grok-shell` | Agent runtime + leader/stdio/headless entry points |
| `crates/codegen/xai-grok-catalog` | Provider/model catalog, discovery, credential handling (fork addition) |
| `crates/codegen/xai-grok-tools` | Tool implementations (terminal, file edit, search, ...) |
| `crates/codegen/xai-grok-workspace` | Host filesystem, VCS, execution, checkpoints |
| `crates/codegen/...` | The rest of the CLI crate closure (config, MCP, markdown, sandbox, ...) |
| `crates/common/`, `crates/build/`, `prod/mc/` | Small shared leaf crates pulled in by the closure |
| `third_party/` | Vendored upstream source (Mermaid diagram stack) |

Unlike upstream, the root `Cargo.toml` is maintained directly in this fork.

## Development

```sh
cargo check -p <crate>        # always target specific crates; full-workspace builds are slow
cargo test -p xai-grok-config # per-crate tests
cargo clippy -p <crate>       # lint config: clippy.toml at the repo root
cargo fmt --all               # rustfmt.toml at the repo root
```

See [`TODO.md`](TODO.md) for tracked follow-up work.

## Roadmap / TODO

Things that do not exist yet:

- [x] **Releases** — prebuilt binaries for Linux (x86_64 + aarch64),
  tag-driven via GitHub Actions; see [`docs/releasing.md`](docs/releasing.md)
- [x] **One-line installer** — [`install.sh`](install.sh) with checksum
  verification
- [x] **Binary/branding rename** — the binary ships as `groky`; config lives
  under `~/.groky` (with automatic migration from `~/.grok`)
- [ ] **groky.dev website** — landing page and hosted documentation (will also
  serve `install.sh`)
- [ ] **macOS release builds** — the GitHub macOS runners hung on the first
  attempt; matrix entries are disabled in `release.yml` pending diagnosis
  (building from source on macOS works)
- [ ] **Windows builds** — currently best-effort/untested
- [ ] **CI breadth** — build/test/lint pipeline beyond the provider-catalog
  check, including the telemetry scan gate

## License

First-party code in this repository is licensed under the **Apache License,
Version 2.0** — see [`LICENSE`](LICENSE).

Third-party and vendored code remains under its original licenses. See:

- [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES) — crates.io / git dependencies,
  bundled UI themes, and **in-tree source ports** (including openai/codex and
  sst/opencode tool implementations)
- [`crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md`](crates/codegen/xai-grok-tools/THIRD_PARTY_NOTICES.md)
  — crate-local notice for the codex and opencode ports (license texts +
  Apache §4(b) change notice)
- [`third_party/NOTICE`](third_party/NOTICE) — vendored Mermaid-stack index

This fork is not affiliated with or endorsed by xAI. "Grok" is a trademark of
its respective owner; this project's changes are provided under Apache-2.0
with change notices per §4(b).
