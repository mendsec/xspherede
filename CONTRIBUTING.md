# Contributing to XSphereDE

Thank you for considering a contribution. XSphereDE is an early prototype, so well-scoped patches, test reports, and design feedback all move it forward.

## Ways to contribute

- **Platform reports.** Run it on your system and open an issue with the result. BSD, non-Mesa drivers, multi-monitor, and HiDPI setups are the least covered.
- **Roadmap work.** See the [roadmap](README.md#roadmap). For anything larger than a small fix, open an issue first so the approach can be agreed before you invest time.
- **Bug fixes and cleanups.** Always welcome.
- **Documentation.** Corrections and clarifications are as valuable as code.

## Development setup

You need Rust 1.88 or newer and a working GPU driver (Vulkan, or OpenGL / GLES).

```bash
git clone https://github.com/mendsec/xspherede.git
cd xspherede
cargo run --locked
```

## Before you open a pull request

Run the same checks as CI:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

If you change `src/shader.wgsl`, the pinned digest in `src/security.rs` must be updated in the same commit, otherwise the integrity test fails by design.

## Project rules

1. **English only.** Code, comments, log messages, UI strings, documentation, and commit messages.
2. **No `unsafe`.** The crate forbids it. If a change appears to need it, open an issue to discuss alternatives.
3. **Keep dependencies few.** Every new crate enlarges the audit surface. Justify additions in the pull request description.
4. **Security-sensitive changes stay discreet.** Do not include exploit details, proofs of concept, or vulnerability analysis in public issues, pull requests, or commit messages. Report privately as described in [SECURITY.md](SECURITY.md), and describe fixes in plain functional terms.
5. **Do not overstate.** Documentation and log messages must describe what the code actually does today, not what it is intended to do later.

## Commit and pull request style

- Use [Conventional Commits](https://www.conventionalcommits.org/) prefixes: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`.
- Keep pull requests focused on one change.
- Add or update tests when you change behavior in `security.rs`, raycasting, or geometry code.
- Note user-visible changes in [CHANGELOG.md](CHANGELOG.md) under `Unreleased`.

## Licensing

By submitting a contribution you agree that it is licensed under the [MIT License](LICENSE), the same license as the project.
