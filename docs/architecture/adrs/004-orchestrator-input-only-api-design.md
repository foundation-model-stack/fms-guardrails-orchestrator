# ADR 004: Orchestrator input only detection API


This ADR documents the design and decisions for the orchestrator APIs.

Orchestrator API can be found [at these github pages](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/).

## Motivation

The guardrails orchestrator is designed to provide an end-to-end guardrails experience. As a part of this, the orchestrator would need to expose calling out to different detectors in isolation from text generation. Since there are different types of detectors segregated by their input requirements and use-cases, we need to expose related APIs in orchestrator to support those different use-cases.


## Decisions


### Nomenclature

1. The endpoints would be named with the primary "task" they do. For example, for the orchestrator endpoint that does both generation and detection in the same call, the task will be called `generation-detection`. For an endpoint which does `detection` as the primary task with a different "modality", the endpoint will include `detection/chat`, where "chat" is the modality.
1. Evidence will be added to responses of each endpoint later on, once we have the detectors under any category that actually is able to provide evidence.
1. All of the endpoints would be prefixed by the modality of the input / output. So for text generation use-cases, the modality would be `text`, for text to image generation, the modality would be `text-image`, for image to image functionalities, the modality would be `image`.
1. All new endpoints would get released under `v2`, as a change from existing APIs, to indicate API change.


### Endpoints

1. Content
    - **Endpoint:** `/api/v2/text/detection/content`
    - **Description:** Endpoint implementing detection task for textual content. This would map to `/text/contents` endpoint of detectors and would target general purpose text content analysis detections. The response from this endpoint would return spans denoting location of the detection.
1. Chat
    - **Endpoint:** `/api/v2/text/detection/chat`
    - **Description:** This endpoint will implement detection task for chat input and will support detectors, exposed via `/api/v1/text/context/chat` endpoint.
1. Document Context
    - **Endpoint:** `/api/v2/text/detection/context`
    - **Description:** This endpoint will implement detection task for document based context detectors, exposed via `/api/v1/text/context/chat` endpoint.
1. Generation Detection
    - **Endpoint:** `/api/v2/text/generation-detection`
    - **Description:** This endpoint will use the input prompt and call the LLM generation and run the requested detectors on the `generated_text`. This endpoint will loosely be similar to the "output detection" API we have currently in the end-to-end guardrails experience. However, one big difference is that, this endpoint will support detectors that need both input prompt and detection together to analyze and provide results.
    - **Notes:**
        - Unlike the end-to-end guardrails experience endpoints, i.e. `/api/v1/task/classification-with-text-generation` and `/api/v1/task/server-streaming-classification-with-text-generation`, this endpoint will not accept a `guardrails_config`. This is because detectors supported by this endpoint work on both input and generated text, where the `generated_text` is produced from the full input `prompt`. We can't provide partial input to detectors; otherwise, the outputs may not be accurate.
        - We would not have `guardrail_config.output` or `guardrail_config.input` here, since this API only works with _both_ input prompt + generated_text.
        - `model_id` in this API refers to LLM `model_id`, similar to existing "guardrails experience"

### API Behavior
1. Each of the endpoints mentioned in this document would accept list of `detector_id` as the input. Each of these detectors are assumed to be of "same" type. If they turn out to be different, then we will throw appropriate 400 error.
   - NOTE: Orchestrator currently does not have a way to map detector-ids with their type / endpoint. In future, we may implement such a mapping to help with error handling.
1. The output of each endpoint will contain a flattened list of outputs from each detector.
1. In the future, to identify which output element in the response belongs to which `detector_id`, we will add `detector_id` as another field in the response object.
1. If any of the endpoints / detectors fail, we will fail the entire request.
1. These new endpoints will not handle server-side streaming and will not accept streaming input.

## Consequences

1. Orchestrator would need to implement API and processing for these new endpoints.
1. Existing endpoints, i.e. `/api/v1/task/classification-with-text-generation` and `/api/v1/task/server-streaming-classification-with-text-generation` do not follow the new paradigm and nomenclature described in this ADR, specially the definition of "task" refered here. However, these will be kept around for API compatibility. We may deprecate and create new endpoint for these.
1. These new endpoints will require changes to how orchestrator request processing workflow is implemented currently and we would need to expand the handlers and tasks.
1. In the existing endpoints, we use a mixed nomenclature, where we refer to "models" for either LLMs or detectors. However, in the new endpoints and API, we will refer to an LLM model ID as `model_id` and use `detector_id` for detectors.

## Status

Accepted
