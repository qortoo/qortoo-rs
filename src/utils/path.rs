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
        let test_func_name = crate::utils::path::get_test_func_name!();
        ret.push_back(test_func_name);
        ret
    }};
}

#[cfg(test)]
macro_rules! get_test_func_name {
    () => {{
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("unknown")
            .to_string();
        let mut ret = crate::utils::path::split_module_path(&thread_name);
        let last = ret.pop_back().unwrap();
        last
    }};
}

#[cfg(test)]
pub(crate) use caller_path;
#[cfg(test)]
pub(crate) use get_test_func_name;

#[cfg(test)]
mod tests_path {
    use tracing::info;

    use super::*;

    #[test]
    fn can_use_path_macros() {
        let path_parts = split_module_path(module_path!());
        assert_eq!(path_parts, vec!["qortoo", "utils", "path", "tests_path"]);
        let caller = caller_path!();
        info!("{caller:?}");
        let func_name = get_test_func_name!();
        info!("func_name: {func_name}");
        assert_eq!(func_name, "can_use_path_macros");
    }
}
