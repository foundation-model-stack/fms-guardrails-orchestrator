# FMS Orchestr8

Orchestrator server for invocation of detectors on text generation input and output [name subject to change]

![LLM Orchestration diagram](docs/architecture/images/llm_detector_orchestration.png "Orchestr8 Diagram")


### Sample requests

1. Guardrails with text generation
```bash
curl -v -H "Content-Type: application/json" --request POST --data '{"model_id": "dummy_model_id", "inputs": "dummy input"}' http://localhost:8033/api/v1/task/classification-with-text-generation
```
2. Guardrails with streaming text generation
```bash
curl -v -H "Content-Type: application/json" --request POST --data '{"sample": "data"}' http://localhost:8033/api/v1/task/server-streaming-classification-with-text-generation
```
3. Health Probe
```bash
curl -v http://localhost:8033/health
```

