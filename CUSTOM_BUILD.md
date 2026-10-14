# Custom EasyTier builds

This branch contains the existing dual-stack exit, Windows DNS and physical-interface binding fixes, and Android application routing changes. Its upstream base is `8428a89d2dabc94c97d370ec607c6ca142473626` (version 2.6.4). It does not yet incorporate the 2.7.0 merge review.

The upstream `main` branch is retained. Use `codex/dual-stack-exit` for these custom builds.

## Build and download

Push code to `codex/dual-stack-exit`, or open **Actions → Custom Windows Linux Android → Run workflow**. After the run succeeds, download the required artifact at the bottom of the run page:

| Artifact | Contents |
| --- | --- |
| `easytier-custom-windows-x86_64` | Windows core, CLI, and required driver files |
| `easytier-custom-linux-x86_64` | Static musl Linux core and CLI |
| `easytier-custom-android-arm64` | Signed Android arm64 APK |

Each artifact includes the source commit and SHA-256 checksums. Artifacts are kept for 7 days; download builds you want to keep. Building does not install programs or change any device configuration. These jobs do not publish GitHub Releases.

Frontend VPN/routing tests and a GUI build run first. Windows and Linux also run the QUIC tunnel tests before compiling their release binaries. Only the architectures above are built automatically.

## Android signing

Repository **Settings → Secrets and variables → Actions** must contain:

- `ANDROID_KEYSTORE_BASE64`: base64-encoded contents of the existing signing keystore.
- `ANDROID_KEYSTORE_PROPERTIES`: contents of its `keystore.properties`, with `storeFile=upload-keystore.jks` (relative to the Android app directory).

The workflow restores signing files only during the Android job. Keep the same signing key to upgrade existing installations. Do not commit signing files, personal network configuration, logs, or operational notes.

Source and workflows are prepared from a clean upstream base; the local operations history is not part of this branch.
