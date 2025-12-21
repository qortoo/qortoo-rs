use std::{env, sync::OnceLock};

pub(crate) const SDK_VER: &str = env!("CARGO_PKG_VERSION");
pub(crate) const SDK_NAME: &str = env!("CARGO_PKG_NAME");
pub(crate) const SDK_HASH: &str = match option_env!("GIT_HASH") {
    Some(hash) => hash,
    None => "unknown",
};

static AGENT: OnceLock<String> = OnceLock::new();
pub fn get_agent() -> &'static str {
    AGENT.get_or_init(|| format!("{SDK_NAME}-{SDK_VER}-{SDK_HASH}"))
}

#[cfg(feature = "tracing")]
static SYNCYAM_RS_OTEL_ENABLED: OnceLock<String> = OnceLock::new();
#[cfg(feature = "tracing")]
pub fn is_otel_enabled() -> bool {
    let enabled = SYNCYAM_RS_OTEL_ENABLED
        .get_or_init(|| env::var("SYNCYAM_RS_OTEL_ENABLED").unwrap_or_else(|_| "".to_string()));
    !enabled.is_empty()
}

#[cfg(test)]
mod tests_constants {
    use tracing::info;

    use super::*;
    #[test]
    fn can_access_constants() {
        info!("SDK_VER: {}", SDK_VER);
        info!("SDK_NAME: {}", SDK_NAME);
        info!("SDK_HASH: {}", SDK_HASH);
    }

    #[test]
    fn can_access_agent() {
        info!("AGENT: {}", get_agent());
    }
}
