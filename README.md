# nexus-tee-toolkit

A Rust template for building verifiable [Nexus](https://talus.network) tools that run inside a Trusted Execution Environment (TEE), producing hardware attestations alongside every tool response.

Follows the same structure as [nautilus-rust](https://github.com/Ashwin-3cS/nautilus-cli) — axum HTTP server, stagex reproducible build, AWS Nitro Enclave attestation via `nautilus-enclave`.

---

## The Problem

The current Nexus trust model:

```
Tool Developer says → "trust my tool, I registered it"
Leader says         → "trust me, I called it correctly"
Onchain stores      → the result (no proof of how it was produced)
```

The Leader is the sole trust point for the entire execution chain. It is trusted to call the right tool, not tamper with inputs, not tamper with outputs, and submit the real result. There is no cryptographic proof that the registered tool binary is what actually ran.

## The Solution

```
Tool Developer ships → reproducible binary in TEE enclave
TEE produces         → attestation: "PCR0 + input_hash + output_hash, signed by hardware"
Leader submits       → result + attestation to onchain
Anyone can verify    → build from source → same PCR0 → attestation valid
                    OR GET /attestation → live PCR0 → compare with registered PCR0
```

The TEE becomes the trust point. The Leader becomes a verifiable coordinator. If the Leader submits a different result than what the attestation proves, anyone can detect it by reading onchain state.

---

## Architecture

```
Onchain (Sui Move)
┌─────────────────────────────────────────────────────────┐
│  DAG Vertex                                             │
│    input: { message: "hello" }                          │
│    tool_fqn: xyz.ashwin.tee.demo@1                      │
│    registered_pcr0: sha384:abc123...  ← NEW FIELD       │
└────────────────────┬────────────────────────────────────┘
                     │ RequestWalkExecutionEvent
                     ▼
Offchain (Leader — closed source)
┌─────────────────────────────────────────────────────────┐
│  1. GET /attestation on tool enclave                    │
│     → live PCR0 from running enclave                    │
│  2. Compare live PCR0 with registered_pcr0              │
│     → mismatch → reject, mark execution failed          │
│  3. POST /tee/demo/invoke with signed headers           │
└────────────────────┬────────────────────────────────────┘
                     │ HTTPS (signed)
                     ▼
Offchain (TEE Tool — this repo)
┌─────────────────────────────────────────────────────────┐
│  Inside enclave:                                        │
│    input_hash  = sha256(input)                          │
│    result      = process(input)                         │
│    output_hash = sha256(result)                         │
│    attestation = NSM.GetAttestation(                    │
│                    user_data: {input_hash, output_hash} │
│                  )                                      │
│    → { result, attestation: { pcr0, report, ... } }    │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
Onchain (Sui Move)
┌─────────────────────────────────────────────────────────┐
│  ProvenValue {                                          │
│    output: result,                                      │
│    tool_fqn: xyz.ashwin.tee.demo@1,                     │
│    attestation: { pcr0, report, input_hash, ... }  ← NEW│
│  }                                                      │
│                                                         │
│  Anyone can verify post-hoc:                            │
│    read ProvenValue → decode report → verify sig        │
│    → extract PCR0 → matches registered_pcr0 ✓           │
│    → extract input_hash → matches DAG input ✓           │
│    → extract output_hash → matches stored output ✓      │
└─────────────────────────────────────────────────────────┘
```

---

## Repo Structure

```
nexus-tee-toolkit/
├── Cargo.toml               ← workspace (aws, init, system)
├── Containerfile            ← stagex reproducible build → .eif
├── Makefile                 ← build, run, stop, logs targets
├── parent_forwarder.sh      ← bridges host TCP:4000 ↔ enclave VSOCK:4000
├── src/
│   ├── aws/                 ← NSM entropy + platform init
│   ├── init/                ← enclave OS boot (mounts, console, entropy)
│   ├── system/              ← syscall wrappers (mount, insmod, socket)
│   └── nexus-tool/          ← axum HTTP server (standalone workspace)
│       ├── Cargo.toml
│       ├── run.sh           ← socat VSOCK bridge + starts server
│       └── src/
│           ├── main.rs      ← router, LogBufferLayer, EnclaveKeyPair init
│           ├── lib.rs       ← AppState, LogBuffer, EnclaveError
│           ├── common.rs    ← HTTP handlers + TEE attestation binding
│           └── walrus/      ← Walrus blob storage integration
```

---

## Running Locally

No enclave required. Outside a Nitro enclave, `nautilus-enclave` returns a mock PCR0 (`aaa...`) — the attestation structure is identical, only the hardware signature is absent.

```bash
# Start the tool
cargo run --manifest-path src/nexus-tool/Cargo.toml

# Ping
curl http://localhost:4000/

# Health check (returns ephemeral public key)
curl http://localhost:4000/health

# Get attestation (PCR0 + ephemeral pubkey)
curl http://localhost:4000/get_attestation

# Invoke the demo tool
curl -X POST http://localhost:4000/tee/demo/invoke \
  -H "Content-Type: application/json" \
  -d '{"message": "hello TEE"}'
```

Response:
```json
{
  "ok": {
    "result": "Processed inside enclave: hello TEE",
    "attestation": {
      "pcr0": "aaa...",
      "raw_cbor_hex": "...",
      "tool_fqn": "xyz.ashwin.tee.demo@1",
      "input_hash": "sha256:...",
      "output_hash": "sha256:...",
      "timestamp": "2026-06-15T10:00:00Z"
    }
  }
}
```

### Walrus Storage (live testnet)

```bash
# Upload JSON — returns blob_id + TEE attestation
curl -X POST http://localhost:4000/walrus/upload-json \
  -H "Content-Type: application/json" \
  -d '{"json": "{\"key\": \"value\"}", "epochs": 1}'

# Read back
curl -X POST http://localhost:4000/walrus/read-json \
  -H "Content-Type: application/json" \
  -d '{"blob_id": "<blob_id from above>"}'

# Verify blob availability
curl -X POST http://localhost:4000/walrus/verify-blob \
  -H "Content-Type: application/json" \
  -d '{"blob_id": "<blob_id>"}'

# View recent enclave logs
curl http://localhost:4000/logs
```

---

## Running in a Real Nitro Enclave

```bash
# Build the enclave image file (requires nitro-cli on EC2)
make

# The PCR0 printed here is what you register onchain.
# nitro-cli output: PCR0 = sha384:<hex>

# Run the enclave
make run

# Forward traffic from host to enclave VSOCK
./parent_forwarder.sh

# Tail enclave logs
make logs

# Stop
make stop
```

Inside the enclave, `nautilus-enclave` calls `/dev/nsm` directly — the PCR0 in every attestation response is the real SHA-384 of the running enclave binary.

---

## Protocol Extension Required

The Nexus Tool Registry Move struct currently stores:
```
{ fqn, url, input_schema, output_schema, description }
```

This toolkit proposes extending it to:
```
{ fqn, url, input_schema, output_schema, description, pcr0, tee_type }
```

- `pcr0`: SHA-384 hex of the enclave binary. Registered at tool deployment time.
- `tee_type`: `"nitro"` (AWS Nitro Enclaves) or `"tdx"` (Intel TDX).

This is a Move contract change — Nexus Maintainer territory. The toolkit is the proof-of-concept that makes the case for this extension.

---

## Path to Decentralizing the Leader

The same pattern applied to the Leader itself:

```
Current:
  Leader binary on Talus Labs servers → trusted blindly

With TEE:
  Leader binary runs inside Nitro enclave
  PCR0 registered onchain
  Every execution emits an attestation
  Anyone can verify the Leader didn't tamper with inputs or outputs
```

The tool TEE template is the proof of concept. The Leader TEE is the end state.
