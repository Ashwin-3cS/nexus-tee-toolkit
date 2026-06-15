# nexus-tee-toolkit

A Rust template for building verifiable [Nexus](https://talus.network) tools that run inside a Trusted Execution Environment (TEE), producing hardware attestations alongside every tool response.

Follows Talus naming conventions (`nexus-sdk`, `nexus-tools`, `nexus-toolkit`) and the same `NexusTool` trait pattern as the standard tools in [nexus-tools](https://github.com/Talus-Network/nexus-tools).

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
│  3. POST /tee/demo with signed headers                  │
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

## Running Locally

No enclave required. `TEE_MODE=mock` produces structurally correct attestations with a clearly-labelled placeholder PCR0.

```bash
# Start the tool (mock mode)
docker compose up

# Call the tool
curl -X POST http://localhost:8080/tee/demo/invoke \
  -H "Content-Type: application/json" \
  -d '{"message": "hello TEE"}'
```

Response:
```json
{
  "ok": {
    "result": "Processed inside enclave: hello TEE",
    "attestation": {
      "pcr0": "mock:0000...",
      "report": "",
      "tool_fqn": "xyz.ashwin.tee.demo@1",
      "input_hash": "sha256:...",
      "output_hash": "sha256:...",
      "timestamp": "2026-06-15T10:00:00Z",
      "tee_type": "mock"
    }
  }
}
```

```bash
# Check live PCR0 (what the Leader would call before invoking)
curl http://localhost:8081/attestation
```

```bash
# Verify an attestation document offline
cargo run --bin verify -- attestation.json
cargo run --bin verify -- attestation.json --pcr0 mock:0000...
```

---

## Running in a Real Nitro Enclave

```bash
# Build the enclave image
docker build -f enclave/Dockerfile -t tee-demo:enclave .

# Convert to EIF (on an EC2 instance with nitro-cli)
nitro-cli build-enclave \
  --docker-uri tee-demo:enclave \
  --output-file tee-demo.eif

# The PCR0 printed here is what you register onchain.
# nitro-cli output: PCR0 = sha384:<hex>

# Run the enclave
nitro-cli run-enclave \
  --eif-path tee-demo.eif \
  --memory 512 \
  --cpu-count 2 \
  --enclave-cid 16

# Set TEE_MODE=nitro in the enclave environment for real attestations.
```

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

## Repo Structure

```
nexus-tee-toolkit/
├── Cargo.toml               ← workspace
├── Dockerfile               ← dev-mode (TEE_MODE=mock)
├── docker-compose.yml       ← docker compose up → :8080/:8081
├── enclave/
│   └── Dockerfile           ← real Nitro enclave image
└── tool/
    ├── Cargo.toml
    ├── build.rs             ← same TOOL_FQN_VERSION pattern as nexus-tools
    ├── tools.json           ← tee: { enabled: true, type: "nitro" }
    └── src/
        ├── main.rs          ← NexusTool impl + /attestation sidecar
        ├── attestation.rs   ← attestation generation (mock + nitro)
        └── verify.rs        ← standalone verifier CLI
```

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
