# Contributing to Mathscape

See the [development guide](docs/development.md) for local setup, targeted tests, and documentation conventions. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Open an issue to discuss substantial changes before implementing them. In your pull request, explain the motivation, link related issues, and list the checks you ran. Keep the change focused and update the relevant docs or tests when behavior changes.

## Checks

Run the checks relevant to your change. The [CI workflow](.github/workflows/ci.yml) runs Rust formatting, Clippy, and workspace tests; frontend type checks, unit tests, and builds; and Playwright browser tests.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm --prefix frontend ci
npm --prefix frontend run typecheck
npm --prefix frontend run test:unit
npm --prefix frontend run build
(cd frontend && npx playwright install --with-deps)
npm --prefix frontend run test:reader
```

Ask a maintainer to add the `ci:full` label if your pull request needs the [cross-platform workflow](.github/workflows/cross-platform.yml) for Linux, macOS, and Windows.

Use the [security policy](SECURITY.md) rather than a public issue for potential vulnerabilities. Remove secrets and private material from any logs or reproductions you share.
