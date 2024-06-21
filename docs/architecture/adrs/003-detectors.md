# ADR 001: Detectors as independent REST servers

## Overview

The watsonx.gov provides many guardrails detectors as services. And the new watsonx.ai opensource orchestration framework provides support for a variety of guardrails (internal or external) detectors.

The guardrails detectors are generally classifiers, that might be based on AI decoder model, pipelines of building blocks like regex, models, vectorDB lookups etc. Independent detectors as services make the watsonx the guardrails framework highly flexible, adaptable and scalable for enterprise solutions. Each detector has their own service and deployment.

Streaming enabled with detectors to address the large input context and and LLM responsiveness issues.

## API Signature

### Endpoint /api/v1/text/contents

Example Curl should look like:

```sh
curl -X 'POST' \
  '<hostname>/api/v1/text/contents' \
  -H 'accept: application/json' \
  -H 'detector-id: en_syntax_slate.38m.hap' \
  -H 'Content-Type: application/json' \
  -d '{
  "contents": [
    "Martians are like crocodiles; the more you give them meat, the more they want"
  ]
}'
```

#### Parameters 
```
detector-id: string - required.
(header)
Example: en_syntax_slate.38m.hap
```

#### Body
```
{
  "contents": [
    "Martians are like crocodiles; the more you give them meat, the more they want"
  ]
}
```
#### Responses

- **200** Successful Response
  - Example:
```json
[
  [
    {
      "start": 14,
      "end": 26,
      "detection": "has_HAP",
      "detection_type": "hap",
      "score": 0.8,
      "text": "test@ibm.com",
      "evidences": []
    }
  ]
]
```

- **404** Resource Not Found
  - Example:
```json
{
  "code": 0,
  "message": "string"
}
```
- **422** Validation Error
    - Example:
```json
{
  "code": 0,
  "message": "string"
}
```
## Status

Approved

## Consequences

- Less standards on how each detector should be structured and deployed; Usage of Kserve can standardize this. [More here.](https://pages.github.ibm.com/ai-foundation/watson-fm-playbooks/guardrails/detectors/KServe/)
- Multi model support; Different types of models can be onboarded to be used as detectiors.