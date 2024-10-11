# ADR 007: Orchestrator Telemetry

The guardrails orchestrator uses [OpenTelemetry](https://opentelemetry.io/) to collect and export telemetry data (traces, metrics, and logs).

The orchestrator needs to collect telemetry data for monitoring and observability. It also needs to be able to trace
spans for incoming requests and across client requests to configured detectors, chunkers, and generation services and 
aggregate detailed traces, metrics, and logs that can be monitored from a variety of observability backends.

## Decision

### OpenTelemetry and `tracing`

The orchestrator and client services will make use of the OpenTelemetry SDK and the [OpenTelemetry Protocol (OTLP)](https://opentelemetry.io/docs/specs/otel/protocol/)
for consolidating and collecting telemetry data across services. The orchestrator will be responsible for collecting
telemetry data throughout the lifetime of a request using the `tracing` crate, which is the de facto choice for logging
and tracing for OpenTelemetry in Rust, and exporting it through the OTLP exporter if configured. The OTLP exporter will
send telemetry data to a gRPC or HTTP endpoint that can be configured to point to a running OpenTelemetry (OTEL) collector.
Similarly, detectors should also be able to collect and export telemetry through OTLP to the same OTEL collector.
From the OTEL collector, the telemetry data can then be exported to multiple backends. The OTEL collector and
any observability backends can all be configured alongside the orchestrator and detectors in a deployment.

### Instrumentation
An incoming request to the orchestrator will initialize a new trace, therefore a trace-id and request should be in
one-to-one correspondence. All important functions throughout the control flow of handling a request in the orchestrator
will be instrumented with the `#[tracing::instrument]` attribute macro above the function definition. This will create
and enter a span for each function call and add it to the trace of the request. Here, important functions refers to any
functions that perform important business logic that may incur significant latency, including all the handler functions
for incoming and outgoing requests. It is up to the discretion of the developer to determine what functions are
"significant" enough to indicate a new span in the trace, but adding a new tracing span can always trivially be done by
just adding the instrument macro.

### Metrics
The orchestrator will aggregate metrics regarding the requests it has received/handled, and annotate the metrics with
span attributes allowing for detailed filtering and monitoring. The metrics will be exported through the OTLP exporter
through the metrics provider. Traces exported through the traces provider can also have R.E.D. (request, error and
duration) metrics attached to them implicitly by the OTEL collector using the `spanmetrics` connector. Both the OTLP
metrics and the `spanmetrics` metrics can be exported to configured metrics backends like Prometheus or Grafana.
The orchestrator will handle a variety of useful metrics such as counters and histograms for received/handled 
successful/failed requests, request and stream durations, and server/client errors. Traces and metrics will also relate
incoming orchestrator requests to respective client requests/responses, and collect more business specific metrics
e.g. regarding the outcome of running detection or generation.

### Configuration
The orchestrator will expose CLI args/env variables for configuring the OTLP exporter:
- `OTEL_EXPORTER_OTLP_PROTOCOL=grpc|http` to set the protocol for all the OTLP endpoints
  - `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` and `OTEL_EXPORTER_OTLP_METRICS_PROTOCOL` to set/override the protocol for
    traces or metrics.
- `--otlp-endpoint, OTEL_EXPORTER_OTLP_ENDPOINT` to set the OTLP endpoint 
  - defaults: gRPC `localhost:4317` and HTTP `localhost:4318`
  - `--otlp-traces-endpoint, OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` and `--otlp-metrics-endpoint, 
    OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` to set/override the endpoint for traces or metrics
    - default to `localhost:4317` for gRPC for all data types, and `localhost:4318/v1/traces`, or `metrics`, for HTTP
- `--otlp-export, OTLP_EXPORT=traces,metrics` to specify a list of which data types to export to the OTLP exporters, separated by a
  comma. The possible values are traces, metrics, or both. The OTLP standard specifies three data types (`traces`, 
  `metrics`, `logs`) but since we use the recommended `tracing` crate for logging, we can export logs as traces and
  not use the separate (more experimental) logging export pipeline.
- `RUST_LOG=error|warn|info|debug|trace` to set the Rust log level.
- `--log-format, LOG_FORMAT=full|compact|json|pretty` to set the logging format for logs printed to stdout. All logs collected as
  traces by OTLP will just be structured traces, this argument is specifically for stdout. Default is `full`.
- `--quiet, -q` to silence logging to stdout. If `OTLP_EXPORT=traces` is still provided, all logs can still be viewed
  as traces from an observability backend.

### Cross-service tracing
The orchestrator and client services will be able to consolidate telemetry and share observability through a common
configuration and backends. This will be made possible through the use of the OTLP standard as well as through the
propagation of the trace context through requests across services using the standardized `traceparent` header. The
orchestrator will be expected to initialize a new trace for an incoming request and pass `traceparent` headers
corresponding to this trace to any requests outgoing to clients, and similarly, the orchestrator will expect the client
to provide a `traceparent` header in the response. The orchestrator will not propagate the `traceparent` to outgoing
responses back to the end user (or expect `traceparent` in incoming requests) for security reasons.

## Status

Proposed

## Consequences

- The orchestrator and client services have a common standard to conform to for telemetry, allowing for traceability
  across different services. There does not exist any other attempts at telemetry standardization that is as widely
  accepted as OpenTelemetry, or have the same level of support from existing observability and monitoring services.
- The deployment of the orchestrator must be configured with telemetry service(s) listening for telemetry exported on
  the specified endpoint(s). An [OTEL collector](https://opentelemetry.io/docs/collector/) service can be used to 
  collect and propagate the telemetry data, or the export endpoint(s) can be listened to directly by any backend that 
  supports OTLP (e.g. Jaeger).
- The orchestrator and client services do not need to be concerned with specific observability backends, the OTEL
  collector and OTLP standard can be used to export telemetry data to a variety of backends including Jaeger,
  Prometheus, Grafana, and Instana, as well to OpenShift natively through the OpenTelemetryCollector CRD.
- Using the `tracing` crate in Rust for logging will treat logs as traces, allowing the orchestrator to export logs
  through the trace provider (with OTLP exporter), simplifying the implementation and avoiding use of the logging
  provider which is still considered experimental in many contexts (it exists for compatibility with non `tracing`
  logging libraries).
- For stdout, the new `--log-format` and `--quiet` arguments add more configurability to format or silence logging.
- The integration of the OpenTelemetry API/SDK into the stack is not trivial, and the OpenTelemetry crates will incur
  additional compile time to the orchestrator.
- The OpenTelemetry API/SDK and OTLP standard are new and still evolving, and the orchestrator will need to keep up
  with changes in the OpenTelemetry ecosystem, there could be occasional breaking changes that will need addressing.