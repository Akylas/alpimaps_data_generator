# Contributing to Cairn

Thanks for looking. Bug reports, fixes, docs and new ideas are all welcome.

## Before you start

* **Bugs and small fixes** — open an issue or go straight to a pull request.
* **Anything larger** (a new step, a new option, a UI rework) — open an issue first so the
  design can be agreed before you spend a weekend on it.
* **Questions** — use [Discussions](https://github.com/Akylas/cairn/discussions), not issues.
* **Security problems** — see [SECURITY.md](SECURITY.md); do not open a public issue.

By contributing you agree your work is released under the [MIT license](LICENSE) and that you
will follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Layout

```
crates/core/    cairn-core: the pipeline, no Tauri dependency
crates/cli/     cairn-cli: the `cairn` command line
src-tauri/      the Tauri shell: commands, event forwarding
src/            the Svelte UI
docs/           user and design documentation
scripts/        packaging and release scripts, and the bathymap generator
config/         default Valhalla and elevation-source configuration
examples/       sample areas
tools/legacy/   the original shell/Python pipeline, kept as a reference
tools/bench/    the benchmarks behind the shipped presets
site/           the project website (static, no build step)
planetiler/ valhalla/ tippecanoe/ ...   git submodules
```

[docs/internals.md](docs/internals.md) explains how the pieces fit and why they are built the
way they are; read it before changing the pipeline.

## Setting up

```shell
git clone --recurse-submodules https://github.com/Akylas/cairn
cd cairn
npm install
```

You need a recent stable Rust toolchain, Node 20+, and — for anything that runs the full
pipeline — Java 21+ plus the native libraries listed in the [README](README.md#requirements).
Unit tests need none of that.

| Task | Command |
| --- | --- |
| Run the app with hot reload | `npm run tauri dev` |
| Run only the UI, against the standalone tile server | `cargo run -p cairn-core --example serve -- output --hold` then `npm run dev` |
| Build the CLI | `cargo build --release -p cairn-cli` |
| Test everything | `cargo test --workspace` |
| Format and lint | `cargo fmt --all` and `cargo clippy --workspace --all-targets` |
| Build the planetiler jar, Valhalla and tippecanoe | `npm run build:tools` |

Valhalla linking is opt-in: with no built `valhalla/build` tree the crates compile without it
and routing reports itself unavailable. Set `VALHALLA_DIR` to link against another build.

## Pull requests

* Keep a change focused; unrelated cleanups go in their own PR.
* Add or update tests. The core crate's tests run in seconds — `cargo test -p cairn-core`.
* If you change an option's default, [docs/pipeline-reference.md](docs/pipeline-reference.md)
  must change with it. A test compares the two, on purpose: an untouched run is meant to give the
  same bytes as the reference pipeline.
* New CLI flags and options belong in the schema in `crates/core/src/steps/options.rs`, which
  generates both the app's form and the command line.
* Comments explain *why*. The code already says what.
* Update [docs](docs/) and the README if behavior a user can see changes.

### Commit messages

[Conventional Commits](https://www.conventionalcommits.org): `feat(cli): …`, `fix(terrain): …`,
`perf:`, `refactor:`, `docs:`, `test:`, `chore:`. Releases are cut from them — `feat` and `fix`
become changelog entries — so the type matters.

## Releasing (maintainers)

Run **Actions → Release → Run workflow**. With `bump: auto` it reads the conventional commits
since the last tag, bumps the version in `package.json`, `Cargo.toml` and `tauri.conf.json`,
writes the changelog, tags, builds every platform, and publishes the draft once all of them
succeed. A failed build takes the tag and the version commit back out.

The planetiler fork's jar is released separately by **Actions → Planetiler jar**.
