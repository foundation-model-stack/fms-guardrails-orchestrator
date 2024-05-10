# FMS Guardrails Orchestr8

Orchestrator server for invocation of detectors on text generation input and output [name subject to change]

![LLM Orchestration diagram](docs/architecture/images/llm_detector_orchestration.png "Orchestr8 Diagram")

## Getting Started

Make sure Rust and Cargo are [installed](https://doc.rust-lang.org/cargo/getting-started/installation.html).

To build and install the binary locally:
```sh
cargo install --path .
```

To run the server locally:
```sh
cargo run --bin fms-guardrails-orchestr8
```

To run tests:
```sh
cargo test
```

To build documenation:
```sh
cargo doc
```

### Sample requests

1. Guardrails with text generation
```bash
curl -v -H "Content-Type: application/json" --request POST --data '{"model_id": "dummy_model_id", "inputs": "dummy input"}' http://localhost:8033/api/v1/task/classification-with-text-generation
```
2. Guardrails with streaming text generation
```bash
curl -v -H "Content-Type: application/json" --request POST --data '{"model_id": "dummy_model_id", "inputs": "dummy input"}' http://localhost:8033/api/v1/task/server-streaming-classification-with-text-generation
```
3. Health Probe
```bash
curl -v http://localhost:8033/health
```

