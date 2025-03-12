# ADR 011: Detector fields

This ADR documents the design and decisions for extending the detector API for various fields, to accomodate various outputs from models in the guardrails ecosystem.

This serves as an extension to [ADR 003 - Detector API design](./003-detector-api-design.md). The detector API can be found [at this Github page](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/?urls.primaryName=Detector+API).

## Motivation

Libraries like the [vllm-detector-adapter](https://github.com/foundation-model-stack/vllm-detector-adapter) provide the detector API and serve LLMs like [Granite Guardian](https://huggingface.co/collections/ibm-granite/granite-guardian-models-66db06b1202a56cf7b079562) and [Llama Guard](https://huggingface.co/meta-llama) as detectors easily thrugh [vLLM](https://github.com/vllm-project/vllm).

As the models undergo development, more information is being provided on model output. Llama Guard will provide "unsafe" categories when input has been categorized as "unsafe" e.g. "unsafe\nS1" [ref](https://huggingface.co/meta-llama/Llama-Guard-3-11B-Vision), and Granite Guardian 3.2 began to provide additional information such as confidence e.g. `"No\n<confidence> High </confidence>` to indicate `No` risk and `High` confidence in that decision [ref](https://github.com/ibm-granite/granite-guardian/blob/main/cookbooks/granite-guardian-3.2/detailed_guide_vllm.ipynb).

At the time of writing, the detector API endpoints return two types of responses:
1. [`/text/contents`](https://foundation-model-stack.github.io/fms-guardrails-orchestrator/?urls.primaryName=Detector+API#/Text/text_content_analysis_unary_handler) returns a list of lists of detections with spans. Each list of detections corresponds to the respective content in `contents` provided in the user request, so `contents` with 2 texts would return a list of 2 lists.
2. Other endpoints return a list of detections, without spans.

Any placement of additional detector fields should account for the two types of responses.

## Assumptions
- This ADR considers fields that apply on each particular detection or decision made by the detector model.
- Knowledge of future model plans is being restricted here, so only a few examples are given with already released model functionalities.

## Decisions
- We will add a new high-level field of `metadata` to account for additional information from detector models. This field will provide a dictionary with string keys and arbitrary values, so that values are not constrained to particular types like strings or floats. This will enable flexibility and is how APIs like [Llama Stack](https://github.com/meta-llama/llama-stack) provides additional information, whether on datasets or models.
    
Example
```json
    {
      "detection_type": "animal",
      "detection": "goose",
      "score": 0.2,
      "evidence": [],
      "metadata":
        {
          "confidence": "High",
          "key": 0.3,
          "categories": ["bird"]
        }
    }
```

- To distinguish `metadata` from the existing `evidence` field, any attributes under `evidence` are meant to help answer: "Why was this decision made?"
- `metadata` will just present information, and the orchestrator will not be altering workflow directions based on any information within the `metadata`. The orchestrator is currently not designed to take any action or decision based on model outputs, as the API is designed to present information to the orchestrator API user or consuming application. The user or application can then decide what to do with the information, whether doing another generation call, masking text, or further presenting the info to that consuming application's users. 
- The updates will affect what endpoints of the detector API return, and changes will be reflected on the orchestrator API as well.
- To keep the experience consistent among various detector API endpoints i.e. `/text/contents` vs. others, any added fields will be on the same level e.g. on the same level as `detection`.

### Alternate Considerations

A few alternate strategies were considered with pros and cons documented.

#### Alternate naming for new high-level field to nest new fields

a. `features`

Example
```json
    {
      "detection_type": "animal",
      "detection": "goose",
      "score": 0.2,
      "evidence": [],
      "features":
        {
          "confidence": "High",
          "key": 0.3,
          "categories": ["bird"]
        }
    }
```
Pros:
- Slightly more descriptive than just `metadata`

Cons:
- `features` might not be appropriate for all attributes
- May be confusing in relation to model ['features' in relation to data](https://en.wikipedia.org/wiki/Feature_(machine_learning))
- Similar to the `metadata` case, addition of `features` could create potential confusion with `evidence`
- Similar to the `metadata` case, arbitrary keys and values will be difficult to validate, but implementations also do not have to validate.

b. `attributes` - This is a more general term than `features` but can be considered more restricting than `metadata`. Not all fields may be considered `attributes` of the decision.

c. `controls` - This concept may be too Granite specific  [ref](https://www.ibm.com/granite/docs/models/granite/). Fields like `confidence` are also not "controlled" or requested by the user.

#### Use current API with `evidence`

Currently, a list of `evidence` can be provided, with arbitrary string attributes as `name` and corresponding string `value` and float `score`, with nested `evidence` as necessary. `value` and `score` may not appropriate for each field or attribute case and can be optional.

Example
```json
    {
      "detection_type": "animal",
      "detection": "goose",
      "score": 0.2,
      "evidence": [
        {
          "name": "confidence",
          "value": "High"
        },
        {
          "name": "categories",
          "value": "bird"
        }
      ]
    }
```

Pros:
- The current API can remain the same
- Generally flexible to various attributes and values that are strings, floats

Cons:
- Not all fields or attributes are necessarily appropriate as `evidence` or explanatory toward the particular detection but may be providing more information 
- `value` is constrained to string and `score` is constrained to float currently. For some fields, `value` may be more appropriate as another data format.

#### Add fields at same level as current fields

Example
```json
    {
      "detection_type": "animal",
      "detection": "goose",
      "score": 0.2,
      "evidence": [],
      "confidence": "High"
    }
```
Pro:
- Similar to the `metadata` case, enables a lot of flexibility
- Potentially slightly less confusion of a completely alternate field than `evidence`, but would still require dilineating what is not `evidence`

Cons:
- More so than the `metadata` case, this will be more difficult to document in the API and allow users to expect particular fields on responses, especially if different detectors provide different fields
- Arbitrary keys and values will be difficult to validate
- Still raises the question of what goes under existing `evidence` or is put at the higher level

## Consequences
- Both detector API users and orchestrator API users will see additional fields reflected with detection results.
- The APIs will handle additional model outputs as model versions are released.
- API users will be able to parse the `metadata` field to receive additional model information.
- Implementers of the detector API can use the `metadata` field to provide additional model information.

## Status

Proposed
