# ADR 005: Design for chat completion support

To enable running guardrails on chat completion use-cases, we want to provide a chat completion API that allows users to run various types of detectors in their chat completion flow.

## Motivation

 The [OpenAI chat completions](https://platform.openai.com/docs/guides/chat-completions) feature has been a popular implementation of using generative models to respond in chat conversations. With various conversation use cases such as customer service, we want to expand our API to not only support applying detectors on text in general, but specifically on chat messages.

## Decisions

### General
- We will use the [OpenAI chat completions API](https://platform.openai.com/docs/api-reference/chat) as our the base API for chat completions. Parameters detailed here can be referenced.
- Different chat completions server solutions like [vllm](https://docs.vllm.ai/en/stable/index.html) may provide [extra parameters](https://docs.vllm.ai/en/stable/serving/openai_compatible_server.html#extra-parameters-for-chat-api). For the orchestrator API to remain more agnostic of particular server implementation, the orchestrator can pass through any additional parameters to the generation server, which can do validation of parameters.
- Guardrails-related API updates would be additive, and we will not modify any OpenAI API elements, like `messages` or `choices`.
    - We will add a `detectors` block in the request, containing `input` and `output` blocks. This will provide flexibility and control, allowing users to provide list of detectors with parameters for both chat completions input and output separately.
    - Here, `input` and `output` does not strictly refer to whether only the `input` to chat completions is provided to the detectors or that only the `output` to chat completions is provided to the detectors. Detectors specified in `output` can potentially take both input and output of a chat completions model.
    - We will add a `detections` block in the response.
- If no input or output detectors were requested, then the `detections.<missing element / input or output>` would not be present in the response.
- If none of input or output detectors are requested, then we will return `422` (similar behavior to other standalone `v2` endpoints).


### Response behavior

#### Assumptions regarding choices
This is particular to the [`choices` array](https://platform.openai.com/docs/api-reference/chat/object#chat/object-choices) on the chat completion response.
1. Each choice is independent.
2. Each choice is generated with the same input.

#### Response
- When output detectors are requested, if the LLM (model used to generate chat completions) doesn't generate `content` in any `message` in `choices`, then we will ignore the `choices` and return a warning about `detections.output`. This was written based on detectors at this time designed to work on `content`, such as text detectors. This may change if new detectors work on tool calls, for example.
- We will run all detectors included in `detectors.output` of the user request on all `choices` (that have `message.content`), unless there are certain detectors that are supposed to be run across choices.
- We will order all detections that include span by `start` and move all the other ones towards the end (by not ordering them in any particular order), but grouping the remaining ones based on `detector_id`.
    - An alternate consideration made here was to not order the detections and just include `detector_id`. However, since detectors are called in parallel, just collecting results may lead to different ordering each call. This ordering will allow processing of detections with spans in order by `start`.


##### Streaming response

The streaming response with guardrails is built off the [chat completion chunk responses](https://platform.openai.com/docs/api-reference/chat/streaming). This applies when both streaming and output detectors are requested.
- Each choice's `delta.content`, if present, will be the `content` used for `detections`. Like the unary case, if there is no `content` overall, a warning will be returned.
- Since detector use will cause content to be "chunked" (e.g. collected into sentences for sentence detectors), to prevent ambiguity, the `delta.role` will be provided on each stream event, and `delta.content` will include the "chunked" text. This will apply to each `choice` in `choices`. Like previous [text generation cases](./002-streaming-response-aggregation.md#end-user-experience), this API will also only return fully processed chunks, to ensure that users receive text back only when all detectors have been run.
- A `detections` block will be added to each stream event. Each processed `choice` will be noted with the inclusion of the choice's index in the original `choices` array by a `choice_index` field, even if there are no particular `results` from running the output detectors.
- For output detectors that run on the contents of an entire output stream, results will be included as `detections` on the second-last event (last one before `[DONE]`) of the stream, regardless of presence of `usage`.
    - Note here that overall `usage` is returned as-is to the user, just adding `detections`. Likewise, any other optional response parameters will be unchanged.
    - An alternate consideration made here was the currently implemented aggregation strategy, where if the user requested any output detectors that required the entire output, the stream events with detection results would only be returned all together. The problem here was the results would then not really be "streamed" back.


#### User interactions

This part gives a brief overview of how we would expect a user to access information from the response object.
- Identifying which choice of potentially multiple choices that results exist for: `detections.output[0].choice_index` (will give index of choice in original `choices`)
- Result info of a given choice: `detections.output[0].results[0].detection_type`

### New elements
- We will add a `warnings` element to show cases where the response is not 4xx, but there are issues in processing.

## Consequences
- Only additive changes to the OpenAI chat completions results will be made in the unary case. In the streaming case, streamed events may need to have updated text in `delta.content`. This can affect token counts per event in `usage`, as well as what `tokens` are included for `logprobs.content` or similar fields.
- Validation and error handling will have to be provided for the cases where there is not expected `content` to run detectors on, raising warnings to the user when appropriate.
- Since the results of various detectors can be included as the results of each `choice`, the inclusion of `detector_id` in each result will help distinguish which detector produced which result.
- Results from input detectors will include `message_index` to indicate which message was processed. In cases where only the last message of the chat history may be processed to limit resource usage, this will help make it obvious to the user which message was processed.
- Similarly, since output detectors will be run on all `choices` provided by the chat completions model, `choice_index` will be provided to inform the user which `choice` that detector result(s) apply for. This will apply regardless if the detector itself may take full or partial input to chat completions model.
- A new result aggregation strategy will need to be implemented to put any detector results that require the full output text from a chat completions model (e.g. all the text of a particular `choice`) on the second-last event.
- For the streaming response, implementation-wise, aggregating the streaming responses will be complicated since currently for the openAI chat completions API, each stream event returns content for only one choice, e.g.
```
data: {"id":"chat-72643462c1164571b8c8a2207dd89dc9","object":"chat.completion.chunk","created":1727139047,"model":"model","choices":[{"index":1,"delta":{"role":"assistant"},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":32,"total_tokens":32,"completion_tokens":0}}

data: {"id":"chat-72643462c1164571b8c8a2207dd89dc9","object":"chat.completion.chunk","created":1727139047,"model":"model,"choices":[{"index":2,"delta":{"role":"assistant"},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":32,"total_tokens":32,"completion_tokens":0}}

data: {"id":"chat-72643462c1164571b8c8a2207dd89dc9","object":"chat.completion.chunk","created":1727139047,"model":"model","choices":[{"index":1,"delta":{"content":"MO"},"logprobs":null,"finish_reason":null}],"usage":{"prompt_tokens":32,"total_tokens":33,"completion_tokens":1}}
```
This means that the orchestrator will have to be able to track chunking and detecting for each choice, potentially with multiple detectors and their respective chunkers.
- Based on `detector_type`, detectors may "filter", or work on partial information, from all the input to chat completions. For example, some detectors may only work on individual `content` messages, while other detectors may work on the entire history of chat messages.

## Status

Proposed
