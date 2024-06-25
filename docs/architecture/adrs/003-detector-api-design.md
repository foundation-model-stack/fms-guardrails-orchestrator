# ADR 003: Detector API design

This ADR documents the design and decisions for the detectors APIs published and integrated into orchestrator. This will also serve the basis of expanding or enhanving the detectors API in future.

## Motivation

This orchestrator is designed to work with various detectors. In the realm of guardrails and trustworthy AI, there can be different types of detectors for different use-cases. From orchestrator perpsective, we want to have the ability to work with a lot of those detector and provide coherent interface to users. Thus, there is a need to provide a common API definitions that individual detectors can expose (based on their use-case) and can get consumed by orchestrator without much changes.

## Decisions

We will have multiple detector APIs divided based primarily on the input requirements and secondary on use-case. For designing the APIs, we will also use common nomenclatures for these inputs and use-cases that are used in other open-source projects in the context of generative AI.

Based on the input requirements, the APIs will be divided into following parts:
1. Text Analysis: This will cover detectors that accept single text as input, and that text can be coming from user or LLM.
1. Generation Analysis: This will cover detectors that needs to work on both input prompt and generated text in combination to provide a singular result.
1. Chat analysis: This will cover the detectors that work on chat history.
1. Context analysis: This will allow integrations with detectors that require context of a prompt, in forms of URL or documents.

### Nomenclature
1. `/text` in the endpoint points to the modality of the input.
1. Content / Contents: This specify any arbitrary input and in the context of `/text/contents`, it denotes text input. Rational behind this selection:
    1. Points to generic input, i.e it is not too specific. So the input can be both prompt or LLM generated text.
    1. Does not have correlated expectation in the output, like if the name was `input`, the  one could expect `output` in response.
    1. Lines up with how text is refered to as in some of the other open source APIs, specially for chat.
1. `detector_id`: This refers to an identifier to a deployment, service, or model id of the detector. It is a way to identify a detector from other.
1. `Context`: Refers to the list of `context_type` objects, in string form allowing user to pass on the context to detectors.
1. `context_type`: Refers to the type of `Context` provided in the API. It can be one of `url` and `doc`. 
1. `detection` (in respons object): Name of the detection, like `EmailAddress`
1. `detection_type`: Type of the detection, like HAP / PII.
1. `score`: Points to the score returned by the detector. It can be confidence, probability etc.

### Endpoints

1. `/api/v1/text/contents` - Text Analysis.
    - Providing detector compution on `contents` (list of string).
    - We are accepting list of string here instead of single string (`content`), to allow batch processing by detectors, which often can be more optimized than singular inputs.
    - Each response in ouput for this endpoint, needs to be in order of the "contents" in input. If there are no detections on any of the inputs, the detector should respond with `[]` response.
1. `/api/v1/text/generation` - Generation Analysis
    - Providing detection computation on both prompt and generated text.
1. `/api/v1/text/context/chat` - Chat Analysis
    - Providing detection computation on chat history.
1. `/api/v1/text/context/doc` - Context Analysis
    - Providing detection computation of content w.r.t the context provided in the input.

Each of above endpoints will also provide an "evidence" block that will allow future integration with evidences for their response.

## Consequences

1. Orchestrator will need to integrate all these different endpoints into detector client to provide support for all these.
1. For `contents` API, already implemented orchestrator workflow, would need to be modified to new API.
1. Each of the detector developer would need to be communicated about new API and its design

## Status

Accepted
