# Entropy CLI

Small command-line tool for the Solana Entropy program.

## Usage

Build or run with Cargo:

```bash
cargo run -p entropy-cli -- --help
```

### Provide mode

Registers a provider (if needed) and listens for `request_with_callback` requests.

```bash
cargo run -p entropy-cli -- provide \
  --entropy-program-id <PROGRAM_ID> \
  --rpc-url http://localhost:8899 \
  --keypair ~/.config/solana/id.json
```

### Request mode

Sends a request to a provider using the simple requester program.

```bash
cargo run -p entropy-cli -- request \
  --provider-id <PROVIDER_ID> \
  --entropy-program-id <PROGRAM_ID> \
  --requester-program-id <SIMPLE_REQUESTER_PROGRAM_ID>
```

## Environment variables

These flags can also be provided via env vars:

- `SOLANA_RPC_URL`
- `SOLANA_KEYPAIR`
- `ENTROPY_PROGRAM_ID`
- `SIMPLE_REQUESTER_PROGRAM_ID`

## Logging

Set `RUST_LOG` to control log level (e.g. `RUST_LOG=info` or `RUST_LOG=debug`).
