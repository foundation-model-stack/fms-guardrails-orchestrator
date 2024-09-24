# ADR 005: Detector Type

This ADR documents the decision of adding the `type` parameter for detectors in the orchestrator config.

## Motivation

The guardrails orchestrator interfaces with different types of detectors. 
Detectors of a given are type are compatible with only a subset of orchestrator endpoints.
In order to reduce changes of misconfiguration, we need a way to map detectors to be used only with compatible endpoints.


## Decision

We decided to add the `type` parameter to the detectors configuration. 
Possible values are `content`, `chat`, `generated` and `context`.
Below is an example of detector configuration.

```yaml
detectors:
    my_detector:
        type: content # Options: content, chat, context, generated
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
    * `content` -> `/api/v1/text/contents`
    * `chat` -> `/api/v1/text/context/chat`
    * `context` -> `/api/v1/text/context/doc`
    * `generated` -> `/api/v1/text/generation`

## Status

Accepted
