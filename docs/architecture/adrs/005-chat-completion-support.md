# ADR 005: Design for chat completion support

To enable running guardrails on chat completion use-cases, we want to provide a chat completion API that allows user ability to run various different types of detectors in their chat completion flow.

## Motivation

[TODO]

## Decisions

### General
1. We will use the [openAI chat completions API](https://platform.openai.com/docs/api-reference/chat) as our the base API for chat completions. Parameters detailed here can be referenced.
1. Guardrails-related updates would be purely additive, and we will not modify any openAI API elements, like messages or choices.
    1. We will add a `detectors` block in the request, containing `input` and `output` blocks. This will provide flexibility and control, allowing users to provide list of detectors with parameters for both chat completions input and output separately.
    1. We will add a `detections` block in the response.
1. If no input or output detectors were requested, then the `detections.<missing element / input or output>` would not be present in the response.
1. If none of input or output detectors are requested, then we will return `422` (similar behavior to other standalone `v2` endpoints).


### Response behavior

#### Assumptions regarding choices
This is particular to the [`choices` array](https://platform.openai.com/docs/api-reference/chat/object#chat/object-choices) on the chat completion response.
1. Each choice is independent.
2. Each choice is generated with the same input.
3. Each requested `output` detector will be run on each choice in `choices`.

#### Response
1. If the LLM (model used to generate chat completions) doesn't generate `content` in any `message` in `choices`, then we will ignore the `choices` and return a warning about `detections.output`.
1. We will run all detectors mentioned in `output` part on all `choices` (that have `message.content`), unless there are certain detectors that are supposed to be run across choices.
1. We will order all detections that include span by `start` and move all the other ones towards the end (by not ordering them in any particular order), but grouping the remaining ones based on `detector_id`.

##### Streaming response
The streaming response with guardrails is built off the [chat completion chunk responses](https://platform.openai.com/docs/api-reference/chat/streaming). This applies when output detectors are requested.
- Each choice's `delta.content`, if present, will be the `content` used for `detections`. Like the unary case, if there is no `content` overall, a warning will be returned.
- Since detector use will cause content to be "chunked" (e.g. collected into sentences for sentence detectors), to prevent ambiguity, the `delta.role` will be provided on each stream event, and `delta.content` will include the "chunked" text. This will apply to each `choice` in `choices`.
- A `detections` block will be added to each stream event. Each processed `choice` will be noted with the inclusion of the choice's `index` by a `choice_index` field, even if there are no particular `results` from running the output detectors.
- For output detectors that run on the contents of an entire output stream, results will be included as `detections` on the second-last event of the stream, regardless of presence of `usage`.
- Implementation-wise, this is complicated since currently for the openAI chat completions API, each stream event returns content for only one choice.


#### User interactions

This part gives a brief overview of how we would expect a user to access information from the response object.
- Identifying which choice of potentially multiple choices that results exist for: `detections.output[0].choice_index`
- Result info of a given choice: `detections.output[0].results[1].detection_type`

### New elements
1. We will add a `warnings` element to show cases where the response is not 4xx, but there are issues in processing.


----- Caveats to rephrase
- detections.input and detections.output are not really about dividing the detectors that work on input vs output. but its about "when" the detectors can work
- output.choice 1 basically says that given choice 1, what is the result of the detector(s). If the detector uses no-input, part of input or full input (like in conversation case), thats a property of the detector.