# ADR 010: Detector fields

This ADR documents the design and decisions for extending the detector API for various fields, to accomodate various outputs from models in the guardrails ecosystem.

This serves as an extension to [ADR 003 - Detector API design](./003-detector-api-design.md). The detector API can be found [at these github pages](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/?urls.primaryName=Detector+API).

## Motivation

Libraries like the [vllm-detector-adapter](https://github.com/foundation-model-stack/vllm-detector-adapter) provide the detector API and serving LLMs like [Granite Guardian](https://huggingface.co/collections/ibm-granite/granite-guardian-models-66db06b1202a56cf7b079562) or [Llama Guard](https://huggingface.co/meta-llama) as detectors easily thrugh [vLLM](https://github.com/vllm-project/vllm).

As the models undergo development, more information is being provided on model output. Llama Guard will provide "unsafe" categories when input has been categorized as "unsafe" e.g. "unsafe\nS1" [ref](https://huggingface.co/meta-llama/Llama-Guard-3-11B-Vision), and Granite Guardian 3.2 began to provide additional information such as confidence [ref](https://github.com/ibm-granite/granite-guardian/blob/main/cookbooks/granite-guardian-3.2/detailed_guide_vllm.ipynb) e.g. `"No\n<confidence> High </confidence>` to indicate `No` risk and `High` confidence in that decision.

At time of writing, the detector API endpoints return two types of responses:
1. [`/text/contents`](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/?urls.primaryName=Detector+API#/Text/text_content_analysis_unary_handler) returns a list of lists of detections with spans. Each list of detections corresponds to the respective content in `contents` provided in the user request, so `contents` with 2 texts would return a list of 2 lists.
2. Other endpoints return a list of detections, without spans.

To keep the experience consistent among various detector API endpoints, any added fields will be on the same level.

## Options

[TODO: This will be split among decision/alternate considerations when more finalized]

1. Use current API - e.g. `evidence`
2. Add new high level field to nest fields under e.g. `metadata` [like llama-stack] or `features` [more specific]
3. Add fields at same level as current fields


## Decisions

### Alternate Considerations

## Consequences


## Status

Proposed
