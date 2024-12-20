# OpenTelemetry

## Traces and Metrics

Traces and metrics are provided for observability of the orchestrator.

[Traces](https://opentelemetry.io/docs/concepts/signals/traces/) provide paths of requests through the application. When tracing is enabled, traces an be viewed in an appropriate backend such as [Jaeger](https://www.jaegertracing.io/), which provides a UI for viewing services and trace operations.

[Metrics](https://opentelemetry.io/docs/concepts/signals/metrics/) capture measurements at runtime of the application. When metrics are enabled, they can be viewed in an appropriate backend such as [Prometheus](https://prometheus.io/), which provides a UI for exploring and graphing metrics.

Example server metrics:
- `incoming_request_count`
- `success_response_count`
- `server_error_response_count`

Example orchestrator client metrics:
- `incoming_request_count`
- `client_response_count`
- `client_request_duration`

## Configuration

Environment variables can be used to configure traces and/or metrics
- Use `OTLP_EXPORT` to provide one of `traces`, `metrics`, or both `traces,metrics`.
- Use `OTEL_EXPORTER_OTLP_ENDPOINT` to configure an endpoint for traces or metrics e.g. `http://collector-svc:4317`

More details and configuration options are noted in the [configuration section of the telemetry ADR](./architecture/adrs/007-orchestrator-telemetry.md#configuration).
