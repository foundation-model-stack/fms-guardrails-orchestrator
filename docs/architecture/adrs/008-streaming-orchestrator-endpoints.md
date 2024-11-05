# ADR 008: Streaming orchestrator endpoints

This ADR documents the design and decisions for streaming orchestrator endpoints.

The orchestrator API can be found [at these github pages](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/).

## Motivation

In [ADR 004](./004-orchestrator-input-only-api-design.md), the design of "input only" detection endpoints was detailed. Currently, those endpoints could only support the "unary" case, where the entire input text is available upfront. For flexibility (example: text is streamed from a generative model that may be available but uncallable through the endpoints with generation), users may still want to call detections on streamed input text.

The orchestrator will then need to support "bidirectional streaming" endpoints, where text (whether tokens, words, sentences) is streamed in, detectors are invoked (and call their respective chunkers, using bidirectional streaming), and text processed with detectors including potential detections is streamed back to the user.

In this ADR, we aim to detail the patterns and behavior to expect for streaming orchestrator endpoints.

## Decisions

### Server streaming or endpoint output streaming
"Server streaming" endpoints existed already prior to the writing of this particular ADR. Streaming response aggregation behavior is documented in [ADR 002](./002-streaming-response-aggregation.md). Data will continue to be streamed back with `data` events, with errors included as `event: error` per the [SSE event format](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events#event_stream_format).

### Client streaming or endpoint input streaming
- We would like to keep input stream events as consistent as possible
    - This means we need to resolve how nested parameters like `detectors` should be inputted, since a simple query or path parameter may not be sufficient. [TODO decision here]
- We will document the "end of stream" message should look like for each endpoint, for the user to indicate the connection should be closed. For example for the OpenAI chat completions API, this looks like a `[DONE]` event.
