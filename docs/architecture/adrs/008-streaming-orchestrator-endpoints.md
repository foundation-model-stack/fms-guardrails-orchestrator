# ADR 008: Streaming orchestrator endpoints

This ADR documents the patterns and behavior expected for streaming orchestrator endpoints.

The orchestrator API can be found [at these github pages](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/).

## Motivation

In [ADR 004](./004-orchestrator-input-only-api-design.md), the design of "input only" detection endpoints was detailed. Currently, those endpoints could only support the "unary" case, where the entire input text is available upfront. For flexibility (example: text is streamed from a generative model that may be available but uncallable through the endpoints with generation), users may still want to call detections on streamed input text.

The orchestrator will then need to support "bidirectional streaming" endpoints, where text (whether tokens, words, sentences) is streamed in, detectors are invoked (and call their respective chunkers, using bidirectional streaming), and text processed with detectors including potential detections is streamed back to the user.


## Decisions

### Server streaming or endpoint output streaming
"Server streaming" endpoints existed already prior to the writing of this particular ADR. Streaming response aggregation behavior is documented in [ADR 002](./002-streaming-response-aggregation.md). Data will continue to be streamed back with `data` events, with errors included as `event: error` per the [SSE event format](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events#event_stream_format).

Parameters in each response event such as `start_index` and `processed_index` will indicate to the user how much of the input stream has been processed for detections, as there might not necessarily be results like positive `detections` for certain portions of the input stream. The `start_index` and `processed_index` will be relative to the entire stream.

### Client streaming or endpoint input streaming
- Any information needed for an entire request, like `detectors` that any detection endpoints will work on, will be expected to be present in the first event of a stream. The structure of stream events expected will be documented for each endpoint.
    - An alternate consideration was using query or path parameters for information needed for an entire request, like `detectors`, but this would be complicated for the nesting that `detectors` require currently, with a mapping of each detector to dictionary parameters.
    - Another alternate consideration was expecting multipart requests, one part with information for the entire request like `detectors` and another part with individual stream events. However, here the content type accepted by the request would have to change.
- Stream closing will be the expected indication that stream events have ended. 
    - An alternate consideration is an explicit "end of stream" request message for each endpoint, for the user to indicate the connection should be closed. For example for the OpenAI chat completions API, this looks like a `[DONE]` event. The downside here is that this particular event's contents will have to be identified and processed differently from other events.

### Separate streaming detection endpoints

To be clear to users, we will start with endpoints that indicate `stream` in the endpoint name. We want to avoid adding `stream` parameters in the request body since this will increase the maintenance of parameters on each request event in the streaming case. Additionally, as detailed earlier, stream endpoint responses will tend to have additional or potentially different fields than their unary counterparts. This point can be altered based on sufficient user feedback.

NOTE: This ADR will not prescribe implementation details, but while the underlying implementation _could_ use [websockets](https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API), we are explicitly not following the patterns of some websocket APIs that require connecting and disconnecting.

## Consequences

- Stream detection endpoints will be separate from current "unary" ones that take entire inputs and return one response. Users then must change endpoints for this different use case.
- The orchestrator can support input or client streaming in a consistent manner. This will enable orchestrator users that may want to stream input content from other sources, like their own generative model.
- Users have to be aware that for input streaming, the first event may need to contain more information necessary for the endpoint. Thus the event message structure may not be exactly the same across events in the stream.

## Status

Accepted
