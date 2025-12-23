mod tests_datatype_builder {
    use qortoo::{Client, DataType, Datatype, DatatypeState};
    use tracing::instrument;

    #[test]
    #[instrument]
    fn can_build_counter() {
        let client = Client::builder(module_path!(), "can_build_counter").build();
        let counter = client
            .create_datatype("counter-1")
            .with_max_memory_size_of_push_buffer(10_000_000)
            .build_counter()
            .unwrap();
        counter.increase_by(42).unwrap();
        assert_eq!("counter-1", counter.get_key());
        assert_eq!(DataType::Counter, counter.get_type());
        assert!(matches!(
            counter.get_state(),
            DatatypeState::DueToCreate | DatatypeState::Subscribed
        ));
        assert_eq!(counter.get_value(), 42);
    }
}
