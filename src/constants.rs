use std::{env, sync::OnceLock};

#[allow(dead_code)]
pub(crate) const SDK_VER: &str = env!("CARGO_PKG_VERSION");
#[allow(dead_code)]
pub(crate) const SDK_NAME: &str = env!("CARGO_PKG_NAME");
#[allow(dead_code)]
pub(crate) const SDK_HASH: &str = match option_env!("GIT_HASH") {
    Some(hash) => hash,
    None => "unknown",
};

#[allow(dead_code)]
static AGENT: OnceLock<String> = OnceLock::new();
#[allow(dead_code)]
pub fn get_agent() -> &'static str {
    AGENT.get_or_init(|| format!("{SDK_NAME}-{SDK_VER}-{SDK_HASH}"))
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
