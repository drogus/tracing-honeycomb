// #![deny(warnings)]
mod telemetry;
mod telemetry_layer;
mod visitor;

#[cfg(test)]
#[macro_use]
#[cfg(test)]
extern crate lazy_static;

pub use crate::telemetry::{SpanId, TraceCtx, TraceId};
pub use crate::telemetry_layer::TelemetryLayer;