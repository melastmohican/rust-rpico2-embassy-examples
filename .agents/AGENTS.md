# Repository Rules

## Pre-commit & Pre-push CI Verification

Before committing and pushing code changes, ALWAYS execute all local CI checks matching `.github/workflows/rust_ci.yml`:

1. **Rust Fmt Check:** `cargo fmt --all -- --check` (or `cargo fmt` to fix formatting)
2. **Check Examples:** `cargo check --examples`
3. **Clippy Check:** `cargo clippy --examples -- -D warnings`
4. **Build Release:** `cargo build --release && cargo build --release --examples`

## Real Hardware Testing Verification

Do NOT commit or push code changes until the example / feature has been run and verified on real hardware by the user.

