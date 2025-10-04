mod tests_datatype_builder {
    use syncyam::{Client, DataType, Datatype, DatatypeState};
    use tracing::instrument;

    #[test]
    #[instrument]
    fn can_build_counter() {
        let client = Client::builder(module_path!(), "can_build_counter").build();
        let counter = client
            .create_datatype(module_path!())
            .build_counter()
            .unwrap();
        counter.increase_by(42);
        assert_eq!(module_path!(), counter.get_key());
        assert_eq!(DataType::Counter, counter.get_type());
        assert_eq!(DatatypeState::DueToCreate, counter.get_state());
        assert_eq!(counter.get_value(), 42);
    }
}
