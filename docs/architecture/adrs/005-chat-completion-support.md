# ADR 005: Design for chat completion support

To enable running guardrails on chat completion use-cases, we want to provide a chat completion API, that allows user ability to run various different types of detectors in their chat completion flow.

## Motivation


## Decisions

### General
1. We will use openai chat completions API as our base chat completion. 
1. Guardrails related updates would be purely additive and we will not modify any openai API elements, like messages or choices.
    1. We will add a `detectors` block in the request which will contain `input` and `output` blocks, allowing users to provide list of detectors for both input and output times separately, providing flexibilty and control.
    1. We will add a `detections` block in the response.
1. If no input or output detectors were requested, then the `detections.<missing element / input or output>` would not be present in the response.
1. If none of input or output detectors are requested, then we will return `422` (similar to how we do it with other standalone `v2` endpoints).

### Request behavior

### Response behavior
1. If LLM doesn't generate `content` in `choices` then we will ignore that and `detections.output` would be `warning`.
1. We will run all detectors mentioned in `output` part on all `choices` (that have `message.content`), unless there are certain detectors that are supposed to be run across choices.

### New elements
1. We will add a `warnings` element to show cases where the response is not 4xx, but we there are issues in processing.


# Rough Notes:

### What choice it is
detections.output[0].choice

### What is the result of given choice
detections.output[0].results[1].detection_type

detections.input and detections.output are not really about dividing the detectors that work on input vs output. but its about "when" the detectors can work 


output.choice 1 basically says that given choice 1, what is the result of the detector(s). If the detector uses no-input, part of input or full input (like in conversation case), thats a property of the detector.

### How shall we order in detections?
We will order all detections that include span by `start` and move all the other ones towards the end (by not ordering them in any particular form), but grouping the remaining ones based on `detector_id`.

### Assumptions
1. Each choice is independent
2. Each choice generated uses same input
3. We will run all requested detectors (output ones) on all choices.