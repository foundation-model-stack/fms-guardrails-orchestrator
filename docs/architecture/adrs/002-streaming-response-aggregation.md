# ADR 002: Streaming Response Behavior and Result Aggregation

This ADR defines the orchestrator strategy of result aggregation and behavior observed by end users of the initial streaming API endpoint (i.e. streaming text generation with classification) when detections are made on streamed output of text generation models. This currently accounts for detections made with detectors implementing the `/text/contents` endpoint.

## Motivation

This orchestrator is designed to work with various detectors. Each detector will have an associated chunker indicated in an end user configuration. Each chunker could chunk different amounts of text e.g. a sentence chunker vs. a paragraph chunker. Each detector could then provide detection responses on different amounts of text. In the unary case, all of these detection responses could be aggregated together and returned in the single response to the end user. 

However, in the streaming case, as frames are being streamed back, the end user will need to receive detection responses corresponding to the text in the streamed frames. Frames still have to be returned in order so that the user can parse the stream of generated text and detections and know how much of the original text generation stream has been processed i.e. what part of the stream that detections have been run on.


## Decisions

### End user experience

1. Only fully processed frames will be returned to the end user. This means that for each (streamed) frame returned to the user, all detector responses for that particular frame's text up to that frame’s index (`processed_index`) will be returned with the frame.
    1. Generated texts in output frames with detections are still expected to be contiguous, meaning the end user can still piece all of the frames together. This aligns with the way various text generation servers provide output.
    2. If any one requested detector fails, the entire request will error.
2. In order to achieve (1) for detectors that may work with different chunkers, e.g. paragraph vs. sentence, results will not be streamed back/returned until the “slowest” detector has processed. “Slowest” may not necessarily just be in terms of time but in terms of amount of text processed (i.e. largest `processed_index`).
3. Like the unary endpoint, for the streaming endpoint, the `input_token_count` will be provided to the user via a `tokenization` call to the LLM when detections are run on input text. When text generation happens via the generation call to the LLM, `input_token_count` is still expected to be provided via the generation call.

### Response aggregation strategy

1. Only chunker calls will provide indices of the text generation server frames processed (`start_index` and `processed_index`). Detector calls are expected to be unary, and responses only indicate the detections on any given chunk from a chunker. Detectors are expected to still give a reply (i.e. empty array) if there are no detections on a given chunk.
    1. On any detector response, the orchestrator will have to associate the chunker indices with each detection response. The orchestrator will update any span start/end on detection responses with chunker indices to be able to indicate to the end user where the detection was in the overall stream.
2. The orchestrator will have to maintain detection responses that are not ready to be streamed back to the user, such as with queues.
    1. Since thresholding of results per detector happens in the orchestrator, this could happen before results for each detector are placed in the queues to be compared to other detectors' results. Note: this does not mean nothing is added to the queue when a result's score is below the threshold. For each processed chunk, the `processed_index` in the whole stream has to be tracked, so in case a result has a score below a threshold or the detector did not return any positive results (like an empty array signifying no detections), the queue can still be used to track when a detector has responded.
3. Initial primary aggregation strategy via maximum `processed_index`
    1. Before aggregation and the streaming back of any frame to the end user, all detector queues will be checked for at least one response. This ensures that no results will be streamed back to the user before detectors (and corresponding chunkers) have responded.
    2. The maximum `processed_index` for frames at the beginning of each queue will be used for reference. For the other detector queues, responses will be aggregated based on `processed_index` up to the reference maximum `processed_index`.
    3. To ensure there are no premature responses e.g. maximum `processed_index` is 42 and another queue has responses at `processed_index` 5, 17, and 34, responses will be “awaited” for that detector until a response with `processed_index` reaches 42 or more. If more than 42, the last response will be left on the queue.
    4. Once all the responses for up to a certain maximum `processed_index` are identified, the detections will be aggregated into a list to be returned in the `token_classification_results` field of the frame to be streamed back. The text of that frame will of course correspond to the text of the frame that had the maximum `processed_index`.
    5. Once a frame is returned/streamed back, the process will repeat to look for at least one queued response from each detector for comparison, until each detector has processed to the end of the stream.
4. Additional aggregation strategies can be implemented in the orchestrator, even if only one is available and implemented to begin with. These strategies could potentially be configurable at deployment.


## Consequences
1. Stream results will be bounded by the “slowest” detector. If any requested detector (and dependent chunker combination) takes a while to respond or potentially never return any responses, results may be kept waiting (until a potential configured overall request timeout). If any requested detector (and dependent chunker combination) processes only a small bit of text at a time (e.g. tokens) while another requested detector (and dependent chunker combination) processes larger bits of text (e.g. paragraphs), the paragraph results will not be provided back yet to the user until the token results have reached a common processed index. This ensures that frames will be contiguous.
2. Because results will be bounded by either the slowest-in-time chunker-detector combination and/or the chunker-detector combination that processes the most amount of text, optimizations for any particular chunker or detector may be limited if end users are calling other less "efficient" detectors.
3. There are edge cases to the initial documented strategy, such as if any chunkers used by the requested detectors provide overlapping chunks of text. This is considered an acceptable risk, as result ordering is considered more important for users processing a stream.
4. This result aggregation logic will be maintained by the orchestrator/orchestrator maintainers, so there is one place for maintenance as opposed to aggregation by each end user of the orchestrator.
5. Any chunker will be expected to output spans, even in a default case of a "chunker" that does not actually chunk but returns the entire document. The start and end of the document will be expected to be returned.


## Status

Accepted
