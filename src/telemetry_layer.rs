use crate::telemetry::{self, BlackholeTelemetry, HoneycombTelemetry, SpanId, Telemetry, TraceCtx};
use crate::visitor::HoneycombVisitor;
use chrono::{DateTime, Utc};
use rand::Rng;
use std::collections::HashMap;
use std::sync::RwLock;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::{layer::Context, registry, Layer};

/// Tracing Subscriber that uses a 'libhoney-rust' Honeycomb client to publish spans
pub struct TelemetryLayer {
    telemetry: Box<dyn Telemetry + Send + Sync + 'static>,
    service_name: String,
    // used to construct span ids to avoid collisions
    pub(crate) instance_id: u64,
    // lazy trace ctx + init time
    span_data: RwLock<HashMap<Id, TraceCtx>>,
}

impl TelemetryLayer {
    /// Create a new TelemetrySubscriber that uses the provided service_name and
    /// a Honeycomb client initialized using the provided 'libhoney::Config'
    pub fn new(service_name: String, config: libhoney::Config) -> Self {
        let telemetry = Box::new(HoneycombTelemetry::new(config));
        Self::new_(service_name, telemetry)
    }

    // for use in tests, discards spans and events
    pub fn new_blakchole() -> Self {
        let telemetry = Box::new(BlackholeTelemetry);
        Self::new_("".to_string(), telemetry)
    }

    pub(crate) fn new_(
        service_name: String,
        telemetry: Box<dyn Telemetry + Send + Sync + 'static>,
    ) -> Self {
        let instance_id = rand::thread_rng().gen();
        let span_data = RwLock::new(HashMap::new());

        TelemetryLayer {
            instance_id,
            service_name,
            telemetry,
            span_data,
        }
    }

    pub(crate) fn record_trace_ctx(&self, trace_ctx: TraceCtx, id: Id) {
        let mut span_data = self.span_data.write().expect("write lock!");
        span_data.insert(id, trace_ctx); // TODO: handle overwrite?
    }

    pub fn eval_ctx<
        'a,
        X: 'a + registry::LookupSpan<'a>,
        I: std::iter::Iterator<Item = registry::SpanRef<'a, X>>,
    >(
        &self,
        iter: I,
    ) -> Option<TraceCtx> {
        let mut path = Vec::new();

        for span_ref in iter {
            let mut write_guard = span_ref.extensions_mut();
            match write_guard.get_mut() {
                None => {
                    let span_data = self.span_data.read().unwrap();
                    match span_data.get(&span_ref.id()) {
                        None => {
                            drop(write_guard);
                            path.push(span_ref);
                        }
                        Some(local_trace_root) => {
                            write_guard.insert(LazyTraceCtx(local_trace_root.clone()));

                            let res = if path.is_empty() {
                                local_trace_root.clone()
                            } else {
                                TraceCtx {
                                    trace_id: local_trace_root.trace_id.clone(),
                                    parent_span: None,
                                }
                            };

                            for span_ref in path.into_iter() {
                                let mut write_guard = span_ref.extensions_mut();
                                write_guard.insert(LazyTraceCtx(TraceCtx {
                                    trace_id: local_trace_root.trace_id.clone(),
                                    parent_span: None,
                                }));
                            }
                            return Some(res);
                        }
                    }
                }
                Some(LazyTraceCtx(already_evaluated)) => {
                    let res = if path.is_empty() {
                        already_evaluated.clone()
                    } else {
                        TraceCtx {
                            trace_id: already_evaluated.trace_id.clone(),
                            parent_span: None,
                        }
                    };

                    for span_ref in path.into_iter() {
                        let mut write_guard = span_ref.extensions_mut();
                        write_guard.insert(LazyTraceCtx(TraceCtx {
                            trace_id: already_evaluated.trace_id.clone(),
                            parent_span: None,
                        }));
                    }
                    return Some(res);
                }
            }
        }

        None
    }

    fn span_id(&self, tracing_id: Id) -> SpanId {
        SpanId {
            tracing_id,
            instance_id: self.instance_id,
        }
    }
}

impl<S> Layer<S> for TelemetryLayer
where
    S: Subscriber + for<'a> registry::LookupSpan<'a>,
{
    fn new_span(&self, attrs: &Attributes, id: &Id, ctx: Context<S>) {
        let span = ctx.span(id).expect("span data not found during new_span");
        let mut extensions_mut = span.extensions_mut();
        extensions_mut.insert(SpanInitAt::new());

        let mut visitor: HoneycombVisitor = HoneycombVisitor(HashMap::new());
        attrs.record(&mut visitor);
        extensions_mut.insert::<HoneycombVisitor>(visitor);
    }

    fn on_record(&self, id: &Id, values: &Record, ctx: Context<S>) {
        let span = ctx.span(id).expect("span data not found during on_record");
        let mut extensions_mut = span.extensions_mut();
        let visitor: &mut HoneycombVisitor = extensions_mut
            .get_mut()
            .expect("fields extension not found during on_record");
        values.record(visitor);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let parent_id = if let Some(parent_id) = event.parent() {
            // explicit parent
            Some(parent_id.clone())
        } else if event.is_root() {
            // don't bother checking thread local if span is explicitly root according to this fn
            None
        } else if let Some(parent_id) = ctx.current_span().id() {
            // implicit parent from threadlocal ctx
            Some(parent_id.clone())
        } else {
            // no parent span, thus this is a root span
            None
        };

        match parent_id {
            None => {} // not part of a trace, don't bother recording via honeycomb
            Some(parent_id) => {
                let initialized_at = Utc::now();

                let mut visitor = HoneycombVisitor(HashMap::new());
                event.record(&mut visitor);

                // TODO: dedup
                let iter = itertools::unfold(Some(parent_id.clone()), |st| match st {
                    Some(target_id) => {
                        let res = ctx
                            .span(target_id)
                            .expect("span data not found during eval_ctx");
                        *st = res.parent().map(|x| x.id());
                        Some(res)
                    }
                    None => None,
                });

                // only report event if it's part of a trace
                if let Some(parent_trace_ctx) = self.eval_ctx(iter) {
                    let event = telemetry::Event {
                        trace_id: parent_trace_ctx.trace_id,
                        parent_id: Some(self.span_id(parent_id.clone())),
                        initialized_at,
                        level: event.metadata().level().clone(),
                        name: event.metadata().name(),
                        target: event.metadata().target(),
                        service_name: &self.service_name,
                        values: visitor.0,
                    };

                    self.telemetry.report_event(event);
                }
            }
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let span = ctx.span(&id).expect("span data not found during on_close");

        // TODO: dedup
        let iter = itertools::unfold(Some(id.clone()), |st| match st {
            Some(target_id) => {
                let res = ctx
                    .span(target_id)
                    .expect("span data not found during eval_ctx");
                *st = res.parent().map(|x| x.id());
                Some(res)
            }
            None => None,
        });

        // if span's enclosing ctx has a trace id, eval & use to report telemetry
        if let Some(trace_ctx) = self.eval_ctx(iter) {
            let mut extensions_mut = span.extensions_mut();
            let visitor: HoneycombVisitor = extensions_mut
                .remove()
                .expect("should be present on all spans");
            let SpanInitAt(initialized_at) = extensions_mut
                .remove()
                .expect("should be present on all spans");

            let now = Utc::now();
            let now = now.timestamp_millis();
            let elapsed_ms = now - initialized_at.timestamp_millis();

            let parent_id = match trace_ctx.parent_span {
                None => span
                    .parents()
                    .next()
                    .map(|parent| self.span_id(parent.id())),
                Some(parent_span) => Some(parent_span),
            };

            let span = telemetry::Span {
                id: self.span_id(id),
                target: span.metadata().target(),
                level: span.metadata().level().clone(), // copy on inner type
                parent_id,
                name: span.metadata().name(),
                initialized_at: initialized_at.clone(),
                trace_id: trace_ctx.trace_id,
                elapsed_ms,
                service_name: &self.service_name,
                values: visitor.0,
            };

            self.telemetry.report_span(span);
        };
    }

    // FIXME: do I need to do something here? I think no (better to require explicit re-marking as root after copy).
    // called when span copied, needed iff span has trace id/etc already? nah,
    // fn on_id_change(&self, _old: &Id, _new: &Id, _ctx: Context<'_, S>) {}
}

struct LazyTraceCtx(TraceCtx);

struct SpanInitAt(DateTime<Utc>);

impl SpanInitAt {
    fn new() -> Self {
        let initialized_at = Utc::now();

        Self(initialized_at)
    }
}

#[derive(Debug)]
struct PathToRoot<'a, S> {
    registry: &'a S,
    next: Option<Id>,
}

impl<'a, S> Iterator for PathToRoot<'a, S>
where
    S: registry::LookupSpan<'a>,
{
    type Item = registry::SpanRef<'a, S>;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next.take()?;
        let span = self.registry.span(&id)?;
        self.next = span.parent().map(|parent| parent.id());
        Some(span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::TraceId;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::runtime::current_thread::Runtime;
    use tracing::instrument;
    use tracing_subscriber::layer::Layer;

    fn explicit_trace_ctx() -> TraceCtx {
        let trace_id = TraceId::new("test-trace-id".to_string());
        let span_id = SpanId {
            tracing_id: Id::from_u64(1234),
            instance_id: 5678,
        };

        TraceCtx {
            trace_id,
            parent_span: Some(span_id),
        }
    }

    #[test]
    fn test_instrument() {
        with_test_scenario_runner(|| {
            #[instrument]
            fn f(ns: Vec<u64>) {
                explicit_trace_ctx().record_on_current_span();
                for n in ns {
                    g(format!("{}", n));
                }
            }

            #[instrument]
            fn g(_s: String) {
                let use_of_reserved_word = "duration-value";
                tracing::event!(
                    tracing::Level::INFO,
                    duration_ms = use_of_reserved_word,
                    foo = "bar"
                );

                assert_eq!(
                    TraceCtx::eval_current_trace_ctx().map(|x| x.trace_id),
                    Some(explicit_trace_ctx().trace_id)
                );
            }

            f(vec![1, 2, 3]);
        });
    }

    // run async fn (with multiple entry and exit for each span due to delay) with test scenario
    #[test]
    fn test_async_instrument() {
        with_test_scenario_runner(|| {
            #[instrument]
            async fn f(ns: Vec<u64>) {
                explicit_trace_ctx().record_on_current_span();
                for n in ns {
                    g(format!("{}", n)).await;
                }
            }

            #[instrument]
            async fn g(s: String) {
                // delay to force multiple span entry (because it isn't immediately ready)
                tokio::timer::delay_for(Duration::from_millis(100)).await;
                let use_of_reserved_word = "duration-value";
                tracing::event!(
                    tracing::Level::INFO,
                    duration_ms = use_of_reserved_word,
                    foo = "bar"
                );

                assert_eq!(
                    TraceCtx::eval_current_trace_ctx().map(|x| x.trace_id),
                    Some(explicit_trace_ctx().trace_id)
                );
            }

            let mut rt = Runtime::new().unwrap();
            rt.block_on(f(vec![1, 2, 3]));
        });
    }

    fn with_test_scenario_runner<F>(f: F)
    where
        F: Fn() -> (),
    {
        let spans = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let cap = crate::telemetry::test::TestTelemetry::new(spans.clone(), events.clone());
        let layer = TelemetryLayer::new_("test_svc_name".to_string(), Box::new(cap));

        let subscriber = layer.with_subscriber(registry::Registry::default());
        tracing::subscriber::with_default(subscriber, f);

        let spans = spans.lock().unwrap();
        let events = events.lock().unwrap();

        // root span is exited (and reported) last
        let root_span = &spans[3];
        let child_spans = &spans[0..3];

        fn expected(k: String, v: libhoney::Value) -> HashMap<String, libhoney::Value> {
            let mut h = HashMap::new();
            h.insert(k, v);
            h
        }

        let expected_trace_id = TraceId::new("test-trace-id".to_string());

        assert_eq!(
            root_span.values,
            expected("ns".to_string(), libhoney::json!("[1, 2, 3]"))
        );
        assert_eq!(root_span.parent_id, explicit_trace_ctx().parent_span);
        assert_eq!(root_span.trace_id, expected_trace_id);

        for (span, event) in child_spans.iter().zip(events.iter()) {
            // confirm parent and trace ids are as expected
            assert_eq!(span.parent_id, Some(root_span.id.clone()));
            assert_eq!(event.parent_id, Some(span.id.clone()));
            assert_eq!(span.trace_id, explicit_trace_ctx().trace_id);
            assert_eq!(event.trace_id, explicit_trace_ctx().trace_id);

            // test that reserved word field names are modified w/ tracing. prefix
            // (field names like "trace.span_id", "duration_ms", etc are ok)
            assert_eq!(
                event.values["tracing.duration_ms"],
                libhoney::json!("duration-value")
            )
        }
    }
}
