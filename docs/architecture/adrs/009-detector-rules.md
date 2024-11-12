# ADR 009: Detector type rules


## Motivation

As mentioned in the [chat completion support ADR](./005-chat-completion-support.md), users may want to apply various [detector types](./006-detector-type.md) on detector use with endpoints that include generation. 

## Decisions


### Rules

[TODO]

### `/text/contents` rules on chat completions

The [OpenAI chat completion messages](https://platform.openai.com/docs/api-reference/chat/create) can be provided by numerous roles. At time of writing, the roles include: `system`, `user`, `assistant`, `tool`, `function`.

- `contents` detectors will not get applied on `function` or `tool` messages. The messages might not make sense here since they could be code. If there are use cases later for this, we can consider applying other modalities of detectors e.g. "code" detectors, or reversing this rule.
- `contents` detectors will only get applied on the last message of the input. We do this to not repeat/re-process chat message history [input of chat-completions], especially where chat history will keep getting added to. Alternatively we always process each `content` [since technically an array can be processed]. This would be more in line with the `chat` detectors that take whole chat history as inputs.

NOTE: If a user-remediable "rule" is broken, validation errors are expected to be returned.

#### Behavior implication on chat completions with detections endpoint:
- On unary output, only messages with applicable roles will be included. This still applies to every choice, where is choice is considered independent of each other.
- On streaming output, the same applies. The only difference is the messages are obtained from `choice.delta.content` instead of `choice.message.content`.


### Strategies of rule application
a. At detector client level (where today, each detector endpoint becomes its own client type). By the time the client is invoked, however, the "context" of the call is lost (i.e. whether this was called with chat completions [chat messages] or just generation [text]). The request passed to the client only has the information needed to call the detector.
b. At orchestrator endpoint level - The detector endpoints will still have to be specifically invoked, but this will allow rule application on the entire request, specific to the request format that the particular orchestrator endpoint expects.

### Notes to organize
Today clients are endpoint-specific and are invoked e.g. `client.text_contents(detector_id, text_contents_request, request_headers)` where `client` is of `TextContentsDetectorClient` type.

The original user request to orchestrator endpoints is currently _not_ passed to detector clients. This would break the current assumption detector clients only have the information necessary to call detectors.


Option (A)

If we want to force the detector client to have the "filter" or rule application, this could look like the following. All of these will imply the `filter...` is _for_ the particular client call.

For input:
`chat_messages = client.filter_chat_messages(chat_messages, roles, indices)`

For unary output:
`chat_completion_messages = client.filter_chat_completion(chat_completion_messages, roles, indices)`
or for streaming output:
`chat_completion_chunk = client.filter_chat_completion_chunk(chat_completion_chunk, roles, indices)`


Option (B)

Complete separation from detector clients. This would mean the detector types would have to be called out. The main difference here is because the output type isn't constrained by the `client` or target detector type, this could be variable and require more functions for different input messages.

For input in `chat/completions-detection`, a function e.g. `handle_chat_completions_detection(user_request)` can call something like `user_request = filter_input_for_text_context(user_request, roles, indices)` One problem here is the `roles` and `indices` would still be particular to the `user_request` format

Note: in implementation, `user_request` could be nested in another object like `task` but writing this out directly for simplicity. 

For unary output
`chat_completions_messages = filter_output_for_text_context(chat_completions_messages)`
or for streaming output
`chat_completion_chunk = filter_streaming_output_for_text_context(chat_completion_chunk)`

For a different detector type
`context_docs = filter_output_for_text_context(context_docs)` . This means we would need at least different impls / type checking
