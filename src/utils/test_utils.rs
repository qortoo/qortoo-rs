pub fn split_module_path(mod_path: &str) -> std::collections::VecDeque<String> {
    mod_path.split("::").map(|s| s.to_string()).collect()
}

macro_rules! caller_path {
    () => {{
        let line = std::panic::Location::caller().line();
        let mut ret = crate::utils::test_utils::split_module_path(module_path!());
        let last = ret.pop_back().unwrap();
        ret.push_back(format!("{last}:{line}"));
        let test_func_name = crate::utils::test_utils::get_test_func_name!();
        ret.push_back(test_func_name);
        ret
    }};
}

macro_rules! get_test_func_name {
    () => {{
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("unknown")
            .to_string();
        let mut ret = crate::utils::test_utils::split_module_path(&thread_name);
        let last = ret.pop_back().unwrap();
        last
    }};
}

macro_rules! get_test_ids {
    () => {{
        let c = get_test_collection_name!();
        let k = get_test_func_name!();
        let r = format!("{c}/{k}");
        (c, k, r)
    }};
}

macro_rules! get_test_collection_name {
    () => {{
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("unknown")
            .to_string();
        let parts: Vec<&str> = thread_name.split("::").collect();
        let mut collection_name = if parts.len() > 1 {
            parts[..parts.len() - 1].join("-")
        } else {
            "unknown".to_owned()
        };
        if collection_name.len() > 47 {
            collection_name.truncate(47);
        }
        collection_name
    }};
}

pub(crate) use caller_path;
pub(crate) use get_test_collection_name;
pub(crate) use get_test_func_name;
pub(crate) use get_test_ids;

#[cfg(test)]
mod tests_path {
    use tracing::info;

    #[test]
    fn can_use_path_macros() {
        let collection_name = get_test_collection_name!();
        let document_key = get_test_func_name!();
        info!("{collection_name} AND {document_key}");
        assert_eq!(collection_name, "utils-test_utils-tests_path");
        assert_eq!(document_key, "can_use_path_macros");
        let (c, d, r) = get_test_ids!();
        assert_eq!(c, collection_name);
        assert_eq!(d, document_key);
        assert_eq!(r, format!("{c}/{d}"));
    }
}
