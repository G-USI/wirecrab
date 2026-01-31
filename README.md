# Wirecrab

AsyncAPI toolkit in Rust with multi-language code generation.

A rewrite of [asyncapi-python](https://github.com/g-usi/asyncapi-python).

## Features

- Parse and validate AsyncAPI 3 specs
- Generate type-safe code for Rust, Python, and TypeScript
- Plugin architecture for wire protocols and codecs (JSON, etc.)
- Proc macro for Rust (`#[asyncapi("spec.yaml")]`)
- CLI for Python/TypeScript code generation

## Project Structure

```
wirecrab/
├── crates/
│   ├── kernel/                      # Core runtime traits
│   ├── spec/                        # AsyncAPI parser and validator
│   ├── codegen/                     # Code generation backends
│   ├── macros/                      # Rust proc macro
│   ├── cli/                         # CLI binary
│   └── contrib/
│       ├── contrib-wire-memory/  # In-memory wire (testing)
│       └── contrib-codec-json/  # JSON codec
├── .githooks/                      # Git hooks (pre-commit)
├── flake.nix                       # Nix dev environment
└── REUSE.toml                      # License configuration
```

## Development

Recommended: [Nix](https://nixos.org/) with flakes enabled.

```bash
nix develop    # Sets up Rust toolchain and git hooks
cargo build    # Build workspace
cargo test     # Run tests
```

## Cross-Compilation

For ESP32 builds, use cross-compilation:

```bash
nix develop .#esp32    # Cross-compile for ESP32 (xtensa-esp32-espidf)
```

This uses [cross-rs](https://github.com/cross-rs/cross) to build for the ESP32 target while providing the ESP32 toolchain and standard library.

## License

Apache-2.0
