# ADR 010: DetectionBatcher & DetectionBatchStream

This ADR documents the addition of two new abstractions to handle batching (fka "aggregation") of streaming detection results. 

1. `DetectionBatcher`
A trait to implement pluggable batching logic for a `DetectionBatchStream`. It includes an associated `Batch` type, enabling implementations to return different types of batches.

2. `DetectionBatchStream`
A stream adapter that wraps multiple detection streams and produces a stream of batches using a `DetectionBatcher`.

## Motivation

To support initial streaming requirements outlined in ADR 002, we implemented the `Aggregator` and `Tracker` components.

1. `Aggregator` handles batching detections and building results. Internally, it is implemented as 3 actors:

    - `AggregationActor`
    Aggregates detections and sends them to the `ResultActor`

    - `GenerationActor`
    Consumes generations from the generation stream, accumulates them, and provides them on-demand to the `ResultActor` to build responses

    - `ResultActor`
    Builds results from detection batches and generations and sends them to result channel

2. `Tracker` wraps a BTreeMap and contains batching logic. It is used internally by the `AggregationActor`.

The primary issue with these components is that they were designed specifically for the *Streaming Classification With Generation* task and lack flexibility to be extended to additional streaming use cases that require batching detections, e.g.
- A use case may require different batching logic
- A use case may need to use different data structures to implement it's batching logic
- A use case may need to return a different batch type
- A use case may need to build a different result type

Additionally, actors are not used in other areas of this codebase and it introduces concepts that may be unfamiliar to new contributors, further increasing the learning curve.

## Decisions

1. The `DetectionBatcher` trait replaces the `Tracker`, enabling flexible and pluggable batching logic tailored to different use cases.

2. The `DetectionBatchStream`, a stream adapter, replaces the `Aggregator`, enabling more flexiblity as it is generic over `DetectionBatcher`.

3. The task of building results is decoupled and delegated to the task handler as a post-batching task. Instead of using an actor to accumulate and own generation/chat completion message state, a task handler can use a shared vec instead, e.g. `Arc<RwLock<Vec<T>>>`, or other approach per use case requirements.

## Notes
1. The existing *Streaming Classification With Generation* batching logic has been re-implemented in `MaxProcessedIndexBatcher`, a `DetectionBatcher` implementation.

## Status

Accepted
