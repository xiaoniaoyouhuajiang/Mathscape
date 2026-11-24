# Contributing to Mathscape

See the [development guide](docs/development.md) for local setup, targeted tests, and documentation conventions. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Open an issue to discuss substantial changes before implementing them. In your pull request, explain the motivation, link related issues, and list the checks you ran. Keep the change focused and update the relevant docs or tests when behavior changes.

## Checks

Run the checks relevant to your change. The [CI workflow](.github/workflows/ci.yml) runs Rust formatting, Clippy, and workspace tests; frontend type checks, unit tests, and builds; and Playwright browser tests.

On pull requests, jobs whose inputs did not change are skipped; a skipped job still reports its check, so required checks pass. All jobs always run on pushes to `main` and manual CI runs.

- **Rust** is skipped when no Rust-relevant files change. InkPad sources and shared frontend build dependencies still trigger it because the desktop crate embeds the built InkPad assets.
- **Frontend** and both **Playwright** jobs are skipped unless the pull request touches `frontend/`, `fixtures/`, `testdata/ink/`, `apps/mathscape-desktop/icons/`, or `.github/workflows/ci.yml`.

Each job has a timeout, so a hung test fails the check instead of holding a runner: see the [development guide](docs/development.md#ci-与发布) for the limits.

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

Linux is covered by the CI workflow above. Ask a maintainer to add the `ci:full` label if your pull request needs the [cross-platform workflow](.github/workflows/cross-platform.yml): it runs `cargo test` on macOS and Windows and builds the Linux `.deb`, macOS `.dmg`, and Windows NSIS installers, which are kept as workflow artifacts for 7 days. Without the label, cross-platform tests run only after merge, on pushes to `main`; there is no nightly run.

## Releases

Maintainers publish a release by pushing a version tag. Pushing a `v*` tag builds the three installers from the tagged commit and creates a GitHub Release with them attached and notes generated from the merged pull requests.

1. Pick a `main` commit whose CI and cross-platform runs passed. The tag build only packages; it does not rerun tests.
2. Tag it with a semantic version and push the tag:

   ```sh
   git tag -a v0.2.0 -m "Mathscape 0.2.0" <commit>
   git push origin v0.2.0
   ```

3. Watch the cross-platform run for the tag. When all three installer builds pass, the release appears on the [Releases page](https://github.com/xiaoniaoyouhuajiang/Mathscape/releases); edit its notes there if needed.

The installers take their version from the tag (`v0.2.0` becomes `0.2.0`). The `version` fields in `Cargo.toml` and `apps/mathscape-desktop/tauri.conf.json` are not changed by the release, so bump them in a pull request before tagging if they should match. If an installer build fails, no release is created: fix the failure and rerun the failed jobs, or delete the tag and tag a fixed commit.

Use the [security policy](SECURITY.md) rather than a public issue for potential vulnerabilities. Remove secrets and private material from any logs or reproductions you share.
