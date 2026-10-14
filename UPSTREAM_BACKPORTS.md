# Selected v2.7.0 backports

This branch retains its v2.6.4 custom base. It is not a complete v2.7.0 upgrade.
The source tag reviewed is `v2.7.0` (`8b7f1f0196edb5b580829ed5f135050fb5853478`).

## Included

| Upstream change | Behavior retained or backported |
| --- | --- |
| [#2500](https://github.com/EasyTier/EasyTier/pull/2500), `282cd92f` | Windows waits for the TUN interface without terminating the instance after ten seconds. |
| [#2546](https://github.com/EasyTier/EasyTier/pull/2546), `56be71c7` | Android Back moves the task to the background so the in-process VPN can continue. |
| [#2345](https://github.com/EasyTier/EasyTier/pull/2345), `5edc4cb1` | Replace quinn-plaintext with upstream's explicitly sized little-endian checksum implementation for consistent behavior across architectures. This is also a prerequisite for ETQ1. |
| [#2565](https://github.com/EasyTier/EasyTier/pull/2565), `e0bdb516` | Bind QUIC checksums to packet numbers on ETQ1, preventing reordered packets from authenticating with a misdecoded number. QUIC tunnels and proxy connections prefer ETQ1 and retry legacy version 1 only after version rejection. Proxy legacy-peer memory expires after one hour. |
| [#2476](https://github.com/EasyTier/EasyTier/pull/2476), `86222771` | Only matching Pong results reset consecutive failures. Relay handshakes preserve routing policy, including replies to forwarded legacy requests. Peer-center reports atomically replace neighbors and invalidate the digest on removal, latency changes and expiry. A full host packet queue drops excess data without blocking the shared packet router. |
| [#2358](https://github.com/EasyTier/EasyTier/pull/2358), `7632cd64` | With latency-first enabled, use a better relay route even when a direct connection exists. |

The first three patches are cherry-picked with upstream attribution. QUIC and peer
changes are adapted to the existing source layout; the portable-core split is not
required. Existing physical-source binding, fail-closed behavior, stopped-endpoint
recovery, dual-stack exit/DNS handling and Android application routing remain in
this branch. The QUIC fallback reuses the same bound endpoint.

ETQ1 requires support at both ends. A new client can still communicate with a
legacy peer through version 1, but that connection does not receive the ETQ1
packet-number protection. These fixes do not provide loss-aware path selection
or eliminate congestion on the underlying Internet connection.

## Central network management: deferred

[#2622](https://github.com/EasyTier/EasyTier/pull/2622) adds a Web management
control plane for network/device membership, automatic addressing, credentials,
ACLs, Gateways and temporary members. It is useful when many devices or users
need enrollment, centralized configuration or credential revocation. It does not
in itself improve the quality of an existing Internet path.

For a small set of fixed, self-managed devices, retaining the current configuration
is simpler. This feature touches 101 files in its upstream commit, requires the
new credential/management interfaces and database migrations, and overlaps the
custom GUI configuration lifecycle. It is therefore deferred rather than partly
copied into this branch. If centralized enrollment becomes necessary, migrate
onto the complete v2.7.0 base and reapply the custom features, with Web database
backup/restore testing, instead of transplanting only the console UI.

## Validation

The custom build workflow runs the GUI tests/build, and transport/peer regression
suites on Windows x64 and Linux x64 before packaging the core and CLI. It also
builds a signed Android arm64 APK. Additional regressions cover ETQ1 packet-number
mismatch, new/legacy stream exchange, bound IPv4/IPv6 legacy fallback, asymmetric
liveness, stale direct relay ACKs (including legacy request flags), topology
invalidation and a saturated host packet queue.

Local validation passed all 46 selected regressions, both individually and in a
single combined test invocation. Two pre-existing memory-heavy proxy bandwidth
stress tests remain ignored; no new functional regression is skipped. The GUI's
25 tests and build passed in CI. Windows x64 and Linux x64 each passed all 46
selected regressions and produced release binaries; the signed Android arm64 APK
also built successfully. All four jobs passed in
[the validation run](https://github.com/kokoneverout/EasyTier/actions/runs/38527001149)
for packaged source commit `6eb47b7bbe5d0fa031ac8e7cb680909d7c496123`.

Downloaded package hashes and architectures were checked. The Linux binaries
are static musl executables; Windows core/CLI start successfully; the APK's
signing certificate matches the existing key. Isolated loopback processes using
the downloaded Windows build and the previous v2.6.4 build established QUIC
connections in both new-to-old and old-to-new directions, with successful
heartbeat round trips. These checks do not measure WAN stability.

The completed run used separate test invocations. The workflow now passes the
same filters to one test harness invocation, verified locally with 46 passes, to
avoid a second workspace compilation observed between the separate invocations.
This workflow-only follow-up does not change the packaged program source. The
release optimization settings are unchanged.

No running service or device configuration is changed by these source commits. Android Back and Windows
network-unavailable startup still need device-level acceptance when these builds
are installed.
