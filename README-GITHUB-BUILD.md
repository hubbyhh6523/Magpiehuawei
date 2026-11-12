# GitHub Actions build pack (Android ARM64)

This is a source bundle prepared for a new repository or branch. It does not contain API credentials, CodeArts cookies, runtime config, or compiled binaries.

## Contents
- `magpie/source/`: upstream Magpie source snapshot at the Box checkout's HEAD; includes `.github/workflows/android-arm64.yml` to build a Go Android/ARM64 artifact.
- `urban-octo-waddle/`: independent Rust gateway source, not integrated into Magpie.
- `acp-bridge/`: independent Python ACP bridge source and setup notes. Credentials are intentionally excluded.

The workflow uses GitHub-hosted Ubuntu and uploads the resulting binary as an Actions artifact. Validate the Go build command and runtime behavior on a real Android ARM64 device before treating the artifact as deployable; Go's `GOOS=android GOARCH=arm64` output may require Android-specific runtime integration.
