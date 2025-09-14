mod tests_datatype_builder {
    use syncyam::{Client, DataType, Datatype, DatatypeState};

    #[test]
    fn can_build_counter() {
        let client = Client::builder(module_path!(), module_path!()).build();
        let counter = client
            .create_datatype(module_path!())
            .with_max_num_of_rollback_transactions(100000)
            .with_max_size_of_rollback_memory(10_000_000)
            .build_counter()
            .unwrap();
        counter.increase_by(42);
        assert_eq!(module_path!(), counter.get_key());
        assert_eq!(DataType::Counter, counter.get_type());
        assert_eq!(DatatypeState::DueToCreate, counter.get_state());
        assert_eq!(counter.get_value(), 42);
    }
}
