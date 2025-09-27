#[cfg(test)]
pub fn split_module_path(mod_path: &str) -> std::collections::VecDeque<String> {
    mod_path.split("::").map(|s| s.to_string()).collect()
}

#[cfg(test)]
macro_rules! caller_path {
    () => {{
        let line = std::panic::Location::caller().line();
        let mut ret = crate::utils::path::split_module_path(module_path!());
        let last = ret.pop_back().unwrap();
        ret.push_back(format!("{last}:{line}"));
        ret
    }};
}

#[cfg(test)]
pub(crate) use caller_path;

#[cfg(test)]
mod tests_path {
    use tracing::info;

    use super::*;

    #[test]
    fn test_split_module_path() {
        let path_parts = split_module_path(module_path!());
        assert_eq!(path_parts, vec!["syncyam", "utils", "path", "tests_path"]);
        let caller = caller_path!();
        info!("{caller:?}");
    }
}
