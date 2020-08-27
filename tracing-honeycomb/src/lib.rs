#![deny(
    warnings,
    missing_debug_implementations,
    missing_copy_implementations,
    missing_docs
)]

//! This crate provides:
//! - A tracing layer, `TelemetryLayer`, that can be used to publish trace data to honeycomb.io
//! - Utilities for implementing distributed tracing against the honeycomb.io backend
//!
//! As a tracing layer, `TelemetryLayer` can be composed with other layers to provide stdout logging, filtering, etc.

mod honeycomb;
mod visitor;

pub use crate::honeycomb::{HoneycombTelemetry, SpanId, TraceId, Sampler, Data};
pub use crate::visitor::HoneycombVisitor;
use rand::{self, Rng};
#[doc(no_inline)]
pub use tracing_distributed::{TelemetryLayer, TraceCtxError};

/// Register the current span as the local root of a distributed trace.
///
/// Specialized to the honeycomb.io-specific SpanId and TraceId provided by this crate.
pub fn register_dist_tracing_root(
    trace_id: TraceId,
    remote_parent_span: Option<SpanId>,
) -> Result<(), TraceCtxError> {
    tracing_distributed::register_dist_tracing_root(trace_id, remote_parent_span)
}

/// Retrieve the distributed trace context associated with the current span.
///
/// Returns the `TraceId`, if any, that the current span is associated with along with
/// the `SpanId` belonging to the current span.
///
/// Specialized to the honeycomb.io-specific SpanId and TraceId provided by this crate.
pub fn current_dist_trace_ctx() -> Result<(TraceId, SpanId), TraceCtxError> {
    tracing_distributed::current_dist_trace_ctx()
}

/// Construct a TelemetryLayer that does not publish telemetry to any backend.
///
/// Specialized to the honeycomb.io-specific SpanId and TraceId provided by this crate.
pub fn new_blackhole_telemetry_layer(
) -> TelemetryLayer<tracing_distributed::BlackholeTelemetry<SpanId, TraceId>, SpanId, TraceId> {
    let instance_id: u64 = 0;
    TelemetryLayer::new(
        "honeycomb_blackhole_tracing_layer",
        tracing_distributed::BlackholeTelemetry::default(),
        move |tracing_id| SpanId {
            instance_id,
            tracing_id,
        },
    )
}

/// Construct a TelemetryLayer that publishes telemetry to honeycomb.io using the provided honeycomb config.
///
/// Specialized to the honeycomb.io-specific SpanId and TraceId provided by this crate.
pub fn new_honeycomb_telemetry_layer(
    service_name: &'static str,
    honeycomb_config: libhoney::Config,
    sampler: Box<dyn Sampler>,
) -> TelemetryLayer<HoneycombTelemetry, SpanId, TraceId> {
    let instance_id: u64 = rand::thread_rng().gen();
    let mut honeycomb_telemetry = HoneycombTelemetry::new(honeycomb_config);
    honeycomb_telemetry.set_sampler(sampler);
    TelemetryLayer::new(
        service_name,
        honeycomb_telemetry,
        move |tracing_id| SpanId {
            instance_id,
            tracing_id,
        },
    )
}
