macro_rules! add_span_event {
    ($name:expr) => {
        opentelemetry::trace::TraceContextExt::span(&opentelemetry::Context::current())
            .add_event($name.to_string(), vec![])
    };
    ($name:expr, $($key:expr => $value:expr),+) => {
        opentelemetry::trace::TraceContextExt::span(&opentelemetry::Context::current())
            .add_event(
                $name.to_string(),
                vec![$(opentelemetry::KeyValue::new($key, $value.to_string())),+]
            )
    };
}

pub(crate) use add_span_event;
