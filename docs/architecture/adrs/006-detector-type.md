# ADR 006: Detector Type

This ADR documents the decision of adding the `type` parameter for detectors in the orchestrator config.

## Motivation

The guardrails orchestrator interfaces with different types of detectors. 
Detectors of a given type are compatible with only a subset of orchestrator endpoints.
In order to reduce changes of misconfiguration, we need a way to map detectors to be used only with compatible endpoints. This would additionally provide a way for us to refer to a particular detector type within the code, without looking at its `hostname` (url) , which can be error prone. Good example for this is validating if certain detector would work with certain orchestrator endpoint or not.


## Decision

We decided to add the `type` parameter to the detectors configuration. 
Possible values are `text_contents`, `text_chat`, `text_generation` and `text_context_doc`.
Below is an example of detector configuration.

```yaml
detectors:
    my_detector:
        type: text_contents # Options: text_contents, text_context_chat, text_context_doc, text_generation
        service:
            hostname: my-detector.com
            port: 8080
            tls: my_certs
        chunker_id: my_chunker
        default_threshold: 0.5
```

## Consequences

1. Reduced misconfiguration risk.
2. Future logic can be implemented for detectors of a particular type.
3. `hostname` no longer needs the full URL, but only the actual hostname.
4. If `tls` is provided, the `https` protocol is used. `http`, otherwise.
5. Not including `type` results in a configuration validation error on orchestrator startup.
6. Detector endpoints are automatically configured based on `type` as follows:
    * `text_contents` -> `/api/v1/text/contents`
    * `text_chat` -> `/api/v1/text/chat`
    * `text_context_doc` -> `/api/v1/text/context/doc`
    * `text_generation` -> `/api/v1/text/generation`

## Status

Accepted