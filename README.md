# gcemeta

[![ci](https://github.com/mechiru/gcemeta/workflows/ci/badge.svg)](https://github.com/mechiru/gcemeta/actions?query=workflow:ci)
[![Rust Documentation](https://docs.rs/gcemeta/badge.svg)](https://docs.rs/gcemeta)
[![Latest Version](https://img.shields.io/crates/v/gcemeta.svg)](https://crates.io/crates/gcemeta)

This library provides access to [GCE metadata service](https://developers.google.com/compute/docs/metadata).

# Example

```rust
use gcemeta::Client;

let client = Client::new();
println!("on gce = {:?}", client.on_gce().await?);
println!("project id = {:?}", client.project_id().await?);
```

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.
