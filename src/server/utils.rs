/*
 Copyright FMS Guardrails Orchestrator Authors

 Licensed under the Apache License, Version 2.0 (the "License");
 you may not use this file except in compliance with the License.
 You may obtain a copy of the License at

     http://www.apache.org/licenses/LICENSE-2.0

 Unless required by applicable law or agreed to in writing, software
 distributed under the License is distributed on an "AS IS" BASIS,
 WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 See the License for the specific language governing permissions and
 limitations under the License.

*/
use std::collections::HashSet;

use http::HeaderMap;
use opentelemetry::trace::{TraceContextExt, TraceId};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Filters a [`HeaderMap`] with a set of header names, returning a new [`HeaderMap`].
pub fn filter_headers(passthrough_headers: &HashSet<String>, headers: HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| passthrough_headers.contains(&name.as_str().to_lowercase()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Returns the [`TraceId`] for this span context.
pub fn get_trace_id() -> TraceId {
    Span::current().context().span().span_context().trace_id()
}
