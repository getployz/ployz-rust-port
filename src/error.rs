use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
struct ContextError {
    context: String,
    source_display: String,
    source: Box<dyn Error + Send + Sync>,
}

impl Display for ContextError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.source_display)
    }
}

impl Error for ContextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(crate) fn context<E>(context: impl Into<String>, source: E) -> anyhow::Error
where
    E: Error + Send + Sync + 'static,
{
    let source_display = source.to_string();
    anyhow::Error::new(ContextError {
        context: context.into(),
        source_display,
        source: Box::new(source),
    })
}

pub(crate) fn anyhow_context(context: impl Into<String>, source: anyhow::Error) -> anyhow::Error {
    let source_display = format!("{source:#}");
    anyhow::Error::new(ContextError {
        context: context.into(),
        source_display,
        source: source.into_boxed_dyn_error(),
    })
}
