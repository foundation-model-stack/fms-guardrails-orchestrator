# ADR 009: Detector type rules

This ADR defines the concept of "rules" on detector types and gives some examples of documented rules and how they should be applied. This ADR is not intending to document rules exhaustively for various detector types.

## Motivation

As mentioned in the [chat completion support ADR](./005-chat-completion-support.md), users may want to apply various [detector types](./006-detector-type.md) on detector use with endpoints that include generation. We want to define some way of guiding how various detector types can be applied with various generation strategies (such as chat message generation vs. text generation), as detector types work on differing amounts of information. [`text/contents` detectors](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/?urls.primaryName=Detector+API#/Text/text_content_analysis_unary_handler) work on an array of text, while [`text/chat` detectors](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/?urls.primaryName=Detector+API#/Text/chat_analysis_unary_handler_api_v1_text_chat_post) work on entire chat histories.

## Decisions

### Rules

The "rules" on detector types are intended to be applied for the purposes of validating the request inputs for detector types and/or updating the request inputs for detector types, when detectors are used in various contexts, such as being called on various generation-with-detection endpoints. Different rules can be applied based on detectors when they are used on user prompts vs. detectors when they are used on output of a generative model. Rules can also specify whether or not detectors should be used on user prompts or outputs of generative models or both.


### `/text/contents` rules on chat completions

The [OpenAI chat completion messages](https://platform.openai.com/docs/api-reference/chat/create) can be provided with numerous roles. At time of writing, the roles include: `system`, `user`, `assistant`, `tool`, `function`.

The following rules apply for detections on chat completions. Namely this is for endpoints that include generation and not standalone detection.

- `contents` detectors will not get applied on `function` or `tool` messages. The messages might not make sense here for detection since they could include code. If there are use cases later for this, we can consider applying other modalities of detectors e.g. "code" detectors, or reversing this rule.
- `contents` detectors will only get applied on the last message of the input. We do this to not repeat/re-process chat message history [input of chat-completions], especially where chat history will keep getting added to. Alternatively we always process each `content` [since technically an array can be processed], but this would be more in line with the `chat` detectors that take whole chat history as inputs.

NOTE: If a user-remediable "rule" is broken, validation errors are expected to be returned.

#### Behavior implication on chat completions with detections endpoint:
- On detections on unary output, only messages with applicable roles will be considered for detection. This still applies to every output choice, where each choice is considered independent of each other.
- On detections on streaming output, the same applies. The only difference is the messages are obtained from `choice.delta.content` instead of `choice.message.content`.


### Strategies of rule application

Various strategies of rule application were considered and listed below. Each is described with rationale of why this was or was not the selected decision.

Today detector clients are detector endpoint-specific and are invoked e.g. `client.text_contents(detector_id, text_contents_request, request_headers)` where `client` is, for example, of `TextContentsDetectorClient` type.

The original user request to orchestrator endpoints is currently _not_ passed to detector clients. This would break the current assumption detector clients only have the information necessary to call detectors.

(a) At the detector client level (where today, each detector endpoint becomes its own client type) - By the time the detector client is invoked, however, the "context" of the call is lost (i.e. whether this detector was called from chat completions [chat messages] or just generation [text]). The request passed to the detector client calls only has the information needed to call the detector.

(b) At the task level - Currently the tasks closely proxy the orchestrator endpoints. Each task includes the user request parameters, such as `detectors`. However, one issue with attempting to apply rules at this level is the `detectors` by this point only have `detector_id` mapped to detector parameters. The particular `detector_type` for each detector is not tracked at the task level. In each "handler" function of the task, the particular detector clients are fetched with the `detector_id` information, with detector calls being invoked.

(c) At the orchestrator endpoint handler level - The detector endpoints will still have to be specifically invoked as mentioned in (b), but this will allow rule application on the entire request, specific to the request format that the particular orchestrator endpoint expects. This can be implemented with or without a `client` invocation, as described below.

#### Handler calls

While ADRs typically do not prescribe implementation, this level of decision-making was determined to be appropriate as to how rules should be applied. The implementation of particular rules is not prescribed, nor are particular function names.

Both of these options below are assumed to be called from the orchestator endpoint handling functions, in line with strategy (c). The first option presents implicit detector endpoints via the `client` invocation. The second options presents this as "transformation" functions.

1. `client` invocation

For detections on input chat messages for an endpoint like `chat/completions-detection`, a function like `handle_chat_completions_detection` (or whatever the high-level endpoint handler function is called) can call `client.convert_chat_completions_request(...)` where the function signature can look like
```
fn convert_chat_completions_request(&self, request: ChatCompletionsRequest, detector_params: DetectorParams, roles: Vec<String>, indices: Vec<i32>) -> ContentAnalysisRequest
```

For detections on output chat completion choice messages, this can look like a call to `client.convert_chat_completion(...)` where the function signature can look like
```
fn convert_chat_completion(&self, response: ChatCompletion, detector_params: DetectorParams, roles: Vec<String>, indices: Vec<i32>) -> ContentAnalysisRequest
```

Here, because the `client` is used, the target output type i.e. `ContentAnalysisRequest` is implied. However, the `client` method will be given full user request information that is not usually necessary to make a detector call.


2. Complete separation from detector clients.

 This would mean the detector types would have to be called out in any functions used to apply the rules. The main difference here is because the output type isn't constrained by the `client` or target detector type, this could be variable and require more functions for different input messages.

 For detections on input chat messages for an endpoint like `chat/completions-detection`, a function like `handle_chat_completions_detection` (or whatever the high-level endpoint handler function is called) can call `convert_chat_completions_request_for_text_contents(...)` where the function signature can look like
```
fn convert_chat_completions_request_for_text_contents(request: ChatCompletionsRequest, detector_params: DetectorParams, roles: Vec<String>, indices: Vec<i32>) -> ContentAnalysisRequest
```

One problem here is the args like `roles` and `indices` would still be particular to the input request of `ChatCompletionsRequest` but the function `convert_chat_completions_request_for_text_contents` would either (1) have to already be chat completions request specific or (2) try to be generic to types, which might become more difficult to maintain.

For a different detector type, the function would likely have to look different, such as `convert_chat_completions_choices_for_text_contents(response: ChatCompletion, ...) -> ContextDocsDetectionRequest`


## Consequences

[TODO]
