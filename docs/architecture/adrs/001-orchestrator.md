# ADR 001: Orchestrator Definition

The orchestrator component in this repository will be part of an architecture that allows for the invocation of various detectors on text generation input and output. One goal of the architecture will be to allow users to easily supply their own detectors.

## Definitions
- Orchestrator: Single server responsible for coordinating the invocation of detectors on text-generation input and output, buffering and chunking text as required for input to detectors, and aggregating responses from detectors for the end user
- Detector: Server responsible for running detection of a certain class of content on text spans
- Router: Server responsible for parsing a request to identify the “route” and forwarding it to the correct individual service based on the request content
- Frame: Single message on a stream (could contain multiple tokens)
- Buffering: Aggregation of text frames (for streaming cases)
- Chunking: Splitting of text in an arbitrary way (not limited to tokenization, necessarily)
- Chunker: Server responsible for chunking

## Decisions with Consequences

Note: Consequences are not their own separate section because there are many decisions documented here. Mapping consequences back to individual decisions via sections was likely harder to follow than integrating.

1. The orchestrator will be an open-sourced REST server.
    1. Any proprietary NLP capabilities including for chunking and detecting will not be in the orchestrator.
    2. Consequence: The orchestrator can be built and maintained in public/open-source. The orchestrator also can be iterated on independent of any chunkers and detectors.
2.  The orchestrator server will perform buffering and chunking (based on the specification of detector(s) requested and by calling out to chunker API) and send unary requests to each detector.
    1. Chunking policies applicable for all detectors will need to be defined, and the orchestrator would need to be aware of different semantic definitions (token, sentence, paragraph, whole-doc initially). In other words, the orchestrator will have to be aware of linguistic spans.
    2. The orchestrator server will need to know the mapping of chunking mechanism (e.g. tokenization) to a given detector.
        1. For chunking policies/mechanisms that involve linguistic semantics (e.g. sentences), this implies that server will need to know (human) language (e.g. Japanese).
        2. Different detector(s) will be able to be used on input (user prompt) versus on generated text frames (e.g. from a text generation server) and can thus supply different chunking policies/mechanisms.
    3. The orchestrator provides a chunker API for chunkers to implement [see Chunkers section for more details].
        1. Consequence: This will allow chunkers to be available and configurable remotely (i.e. doesn’t have to be local model or even local service), as long as the router knows about the chunker.
        2. Consequence: The orchestrator server code does not have to be in the same programming language as chunker implementations.
3. The orchestrator server will aggregate unary detector responses and re-map output span results from detectors into the overall output stream provided to the end user.
    1. Consequence: The orchestrator server will be more complicated than if the detector server(s) are expected to do input buffering and chunking and output streams.
    2. For the streaming endpoint/case, the orchestrator will stream back frames based on what all requested detectors have processed. 
        1. Consequence: If any requested detector requires a whole document to be processed, the orchestrator will not return results to the end user until the whole document is processed (i.e. result looks unary).
4. The orchestrator will not do routing.
    1. For all of chunkers, detectors, and text generation models, the orchestrator will only take IDs and types, then request the router component for information on (1) whether the chunker / detector / text generation model is available and (2) where to send the request to and get results from chunker / detector / text generation model.
    2. Configuration for which type of chunk (e.g. token, sentence, whole document) will be maintained by an end user or an operator deploying all components (orchestrator, chunker, detectors).
        1. This can be a simple configmap read not maintained by the orchestrator. Importantly, there is no in-memory registration.
        2. This will have to be updated when any new detector server or chunker is added.
        3. Consequence: The orchestrator absolves responsibility of language detection via the model ID paradigm i.e. every detector ID will supply a corresponding chunker ID in the configuration. Thus on request, an end user has to use detector ID(s) appropriate to the language they are expecting (whether on the user prompt or generated text).
4. The orchestrator will be responsible for filtering/thresholding results from individual detectors. For now, default thresholds will be tracked in the configuration of detector servers read by the orchestrator.
    1. Besides potential filtering of individual results, any results from a detector will be assumed to be the “results” returned to the end user. “results” is in quotes since this information will be potentially combined with text generation server result information by the orchestrator, but importantly the results themselves (such as scores or texts) will not be altered by the orchestrator.

### Chunker
Note: The orchestrator will provide an API for chunkers, but chunker implementations will not be provided with the orchestrator. The following decisions are decisions that allow the orchestrator to work with chunker(s).

1. A chunker will be a server providing chunking capabilities, e.g. but not limited to sentence detection, and essentially implementing the “chunking policy” required by the detector. A generic chunking example is chunking on code (e.g. functions, comments).
2. Different chunkers (implementing the policies) would be identified by a “chunker-id” or “model-id”. These individual chunkers can support one or more languages. The detector only needs to provide which “chunker” it is compatible with.
3. Programming language choices for a given chunker would depend on the choice of packages used for tokenization.
4. Streamed chunker results will be directly concatenable. While this may be a limitation, this is in line with many text generation servers' stream results, simplifies tracking of text in the orchestrator, and does not make assumptions about any text, e.g. whitespace, for individual detectors.


### Detector
Note: The orchestrator will provide an API for detectors, but detector implementations will not be provided with the orchestrator. The following decisions are decisions that allow the orchestrator to work with detector(s).

1. Detectors will be REST servers.
2. The detector API will be unary only i.e. not require streaming or handle streaming requests.
    1. Each detector will be able to select its "chunking policy" from a configured/pluggable registry in the orchestrator (token, sentence, paragraph, whole-doc initially).
    2. Consequence: The detector API will be simpler than if the detector server(s) are expected to do input buffering and chunking and output streams. Individual(s)/team(s) contributing detector(s) will not be responsible for this logic. This will likely allow integration of more detectors more easily.
3. In the case where the end user invokes a streaming implementation through the orchestrator, the text passed from chunkers to detectors will be assumed to be directly concatenable for tracking purposes, like streamed text generation server results.
    1. Consequence: Detectors that will be affected e.g. by whitespace surrounding chunks like sentences must account for this.


## Status

Accepted
