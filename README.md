# devin-connect

Source for the customer-deployed Devin Gateway and the tunnel protocol it
speaks to the Devin-side edge.

- `gateway/` — the gateway binary: HTTP CONNECT proxy with a default-deny
  allowlist and an outbound-only TLS tunnel. See [gateway/README.md](gateway/README.md)
  for the deployment model, config template, and token rotation.
- `tunnel/` — the wire protocol (hello exchange, yamux multiplexing,
  WebSocket carrier) shared by both ends of the tunnel.
- `common/` — listener helpers, the admin HTTP surface, Prometheus text
  rendering, and logging setup.

## Building

```
cargo build --locked --release -p gateway
cargo test --locked
docker buildx build . --target gateway
```

`deny.toml` holds the license allowlist for `cargo deny check licenses`.

## Licensing

The workspace crates are licensed under Apache-2.0; see [LICENSE](LICENSE).
The Docker image includes the project license at `/licenses/devin-connect/LICENSE`
and dependency licenses and upstream notices under `/licenses/third-party/`.
The build generates the Rust dependency bundle with pinned `cargo-about`, using
`about.toml` and `about.hbs`, preserves resolved crates' original license and
notice files, and includes the Rust standard library, libunwind, musl, and tini
notices. Tini is built from pinned source against musl.

When changing the Rust image version, review `RUST_LLVM_REV` and the runtime
notices together. Rust 1.88 and the Bookworm toolchain both use musl 1.2.3.
Run the notice-collector test with:

```
python3 -B -m unittest discover -s scripts -p 'test_*.py'
```
