# ADR 007: Orchestrator Telemetry

The guardrails orchestrator uses OpenTelemetry to collect and export telemetry data (traces, metrics, and logs).

The orchestrator needs to collect telemetry data for monitoring and observability. It also needs to be able to trace
spans for incoming requests and across client requests to configured detectors, chunkers, and generation services and 
aggregate details traces, metrics, and logs that can be monitored from a variety of observability backends.

## Decision

### OpenTelemetry and `tracing`

The orchestrator and client services will make use of the OpenTelemetry SDK and the OTLP standard for consolidating
and collecting telemetry data across services. The orchestrator will be responsible for collecting telemetry data
throughout the lifetime of a request using the `tracing` crate, which is the de facto choice for logging and tracing
for OpenTelemetry in Rust, and exporting it through the OTLP exporter if configured. The OTLP exporter will send 
telemetry data to a gRPC or HTTP endpoint that can be configured to point to a running OTEL collector.
Similarly, detectors should also be able to collect and export telemetry through OTLP to the same OTEL collector.
From the OTEL collector, the telemetry data can then be exported to multiple backends. The OTEL collector and
any observability backends can all be configured alongside the orchestrator and detectors in a deployment.

### Instrumentation
An incoming request to the orchestrator will initialize a new trace, therefore a trace-id and request should be in
one-to-one correspondence. All important functions throughout the control flow of handling a request in the orchestrator 
will be instrumented with the `#[tracing::instrument]` attribute macro. This will create and enter a span for each 
function call and add it to the trace of the request. Here, important functions refers to any function that performs
important business logic or that may incur significant latency, this includes all the handler functions for incoming
and outgoing requests.

### Metrics
The orchestrator will aggregate metrics regarding the requests it has received/handled, and annotate the metrics with
span attributes allowing for detailed filtering and monitoring. The metrics will be exported through the OTLP exporter
through the metrics provider. Traces exported through the traces provider can also have R.E.D. (request, error and
duration) metrics attached to them implicitly by the OTEL collector using the `spanmetrics` connector. Both the OTLP
metrics and the `spanmetrics` metrics can be exported to configured metrics backends like Prometheus or Grafana.
The orchestrator will handle a variety of useful metrics such as counters and histograms for received/handled 
successful/failed requests, request and stream durations, and server/client errors.

### Configuration
The orchestrator will expose CLI args/env variables for configuring the OTLP exporter endpoint(s) 
(e.g. `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317`) and protocol(s) (e.g. `OTEL_EXPORTER_OTLP_PROTOCOL=grpc`), 
as well as the ability to enable trace (includes logs) and metrics OTLP exporting (e.g. `OTLP_EXPORT=traces,metrics`.
Alongside this, the orchestrator will also allow for configuring the Rust log level (e.g. `RUST_LOG=debug`) and 
log format to use for logs printed to stdout (e.g. `STDOUT_LOG=pretty`). Printing logs to stdout can also be disabled
(e.g. `STDOUT_LOG=none`). The OTEL collector can be configured to receive from the OTLP endpoint(s) and export to
observability backends. Examples of a complete configuration for the orchestrator and OTEL collector can be found in
the playbooks [LINK HERE] and [LINK HERE] as Docker Compose files and as an OpenShift deployment.

### Cross-service tracing
The orchestrator and client services will be able to consolidate telemetry and share observability through a common
configuration and backends. This will be made possible through the use of the OTLP standard as well as through the
propagation of the trace context through requests across services using the standardized `traceparent` header. The
orchestrator will make use of an incoming request's `traceparent` for initializing a trace context for a request if
it exists, or else will initialize a new trace. 

## Status

Proposed

## Consequences

- The orchestrator and client services have a common standard to conform to for telemetry, allowing for traceability
  across different services.
- The orchestrator and client services do not need to be concerned with specific observability backends, the OTEL
  collector and OTLP standard can be used to export telemetry data to a variety of backends including Jaeger,
  Prometheus, Grafana, and Instana, as well to OpenShift natively through the OpenTelemetryCollector CRD.
- Using the `tracing` crate in Rust for logging will treat logs as traces, allowing the orchestrator to export logs
  through the trace provider (with OTLP exporter), simplifying the implementation and avoiding use of the logging
  provider which is still considered experimental in many contexts (it exists for compatibility with non `tracing`
  logging libraries).
- The integration of the OpenTelemetry API/SDK into the stack is not trivial, and the OpenTelemetry crates will incur
  additional compile time to the orchestrator.
- The OpenTelemetry API/SDK and OTLP standard are new and still evolving, and the orchestrator will need to keep up
  with changes in the OpenTelemetry ecosystem, there could be occasional breaking changes that will need addressing.