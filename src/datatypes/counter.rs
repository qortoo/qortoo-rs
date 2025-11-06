use std::sync::Arc;

use crate::{
    DatatypeError, IntoString,
    datatypes::{
        common::{ReturnType, datatype_instrument},
        crdts::Crdt,
        datatype::DatatypeBlanket,
        transactional::{TransactionContext, TransactionalDatatype},
    },
    errors::BoxedError,
    operations::Operation,
};

/// A counter is a conflict-free datatype that supports increment operations.
#[derive(Clone)]
pub struct Counter {
    datatype: Arc<TransactionalDatatype>,
    tx_ctx: Arc<TransactionContext>,
}

impl Counter {
    pub(crate) fn new(datatype: Arc<TransactionalDatatype>) -> Self {
        Counter {
            datatype,
            tx_ctx: Default::default(),
        }
    }

    datatype_instrument! {
    /// Increases the counter by the specified delta value.
    ///
    /// Returns the new counter-value after the increment.
    /// This operation is conflict-free and can be safely called concurrently.
    ///
    /// # Arguments
    ///
    /// * `delta` - The amount to increase the counter by (can be negative for decrease)
    ///
    /// # Returns
    ///
    /// The new counter-value after applying the increment
    ///
    /// # Examples
    ///
    /// ```
    /// # use syncyam::{Client, Counter, DatatypeState};
    /// let client = Client::builder("doc-example", "increase_by-test").build();
    /// let counter = client.create_datatype("test-counter").build_counter().unwrap();
    /// assert_eq!(counter.increase_by(5), 5);
    /// assert_eq!(counter.increase_by(-2), 3);
    /// ```
    pub fn increase_by(&self, delta: i64) -> Result<i64, DatatypeError> {
        let op = Operation::new_counter_increase(delta);

        let ret = self
            .datatype
            .execute_local_operation_as_tx(self.tx_ctx.clone(), op)?;
        match ret {
            ReturnType::Counter(cv) => Ok(cv),
            _ => Err(DatatypeError::FailedToExecuteOperation("unexpected return type".into()))
        }
    }}

    /// Increases the counter by 1.
    ///
    /// This is a convenience method equivalent to `increase_by(1)`.
    ///
    /// # Returns
    ///
    /// The new counter-value after incrementing by 1
    ///
    /// # Examples
    ///
    /// ```
    /// # use syncyam::{Client, Counter, DatatypeState};
    /// let client = Client::builder("doc-example", "increase-test").build();
    /// let counter = client.create_datatype("test-counter").build_counter().unwrap();
    /// assert_eq!(counter.increase(), 1);
    /// assert_eq!(counter.increase(), 2);
    /// ```
    pub fn increase(&self) -> Result<i64, DatatypeError> {
        self.increase_by(1)
    }

    /// Gets the current counter-value without modifying it.
    ///
    /// # Returns
    ///
    /// The current counter-value
    ///
    /// # Examples
    ///
    /// ```
    /// # use syncyam::{Client, Counter, DatatypeState};
    /// let client = Client::builder("doc-example", "get_value-test").build();
    /// let counter = client.create_datatype("test-counter").build_counter().unwrap();
    /// assert_eq!(counter.get_value(), 0);
    /// counter.increase();
    /// assert_eq!(counter.get_value(), 1);
    /// ```
    pub fn get_value(&self) -> i64 {
        let mutable = self.datatype.mutable.read();
        let Crdt::Counter(c) = &mutable.crdt;
        c.value()
    }

    datatype_instrument! {
    /// Executes multiple operations atomically within a transaction.
    ///
    /// If the transaction function returns an error, all operations within
    /// the transaction are rolled back, leaving the counter unchanged.
    ///
    /// # Arguments
    ///
    /// * `tag` - A descriptive label for the transaction
    /// * `tx_func` - Function containing the operations to execute atomically
    ///
    /// # Returns
    ///
    /// `Ok(())` if the transaction succeeded, `Err(DatatypeError)` otherwise
    ///
    /// # Examples
    ///
    /// ```
    /// # use syncyam::{Client, Counter, DatatypeState};
    /// let client = Client::builder("doc-example", "transaction-test").build();
    /// let counter = client.create_datatype("test-counter").build_counter().unwrap();
    ///
    /// // Successful transaction
    /// let result = counter.transaction("batch-update", |c| {
    ///     c.increase_by(10);
    ///     c.increase_by(5);
    ///     Ok(())
    /// });
    /// assert!(result.is_ok());
    /// assert_eq!(counter.get_value(), 15);
    ///
    /// // Failed transaction - changes are rolled back
    /// let result = counter.transaction("failing-update", |c| {
    ///     c.increase_by(100);
    ///     Err("something went wrong".into())
    /// });
    /// assert!(result.is_err());
    /// assert_eq!(counter.get_value(), 15); // unchanged
    /// ```
    pub fn transaction<T>(
        &self,
        tag: impl IntoString,
        tx_func: T,
    ) -> Result<(), DatatypeError>
    where
        T: FnOnce(Self) -> Result<(), BoxedError> + Send + Sync + 'static,
    {
        self.datatype.check_writable()?;
        let this_tx_ctx = Arc::new(TransactionContext::new(tag));
        let this_tx_ctx_clone = this_tx_ctx.clone();
        let do_tx_func = move || {
            let mut counter_clone = self.clone();
            counter_clone.tx_ctx = this_tx_ctx_clone.clone();
            match tx_func(counter_clone) {
                Ok(_) => Ok(()),
                Err(e) => Err(DatatypeError::FailedTransaction(e)),
            }
        };
        self.datatype.do_transaction(this_tx_ctx, do_tx_func)
    }}
}

impl DatatypeBlanket for Counter {
    fn get_core(&self) -> &TransactionalDatatype {
        self.datatype.as_ref()
    }
}

#[cfg(test)]
mod tests_counter {
    use tracing::{Span, info_span, instrument};
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    use crate::{
        DataType,
        datatypes::{
            common::new_attribute, counter::Counter, datatype::Datatype,
            transactional::TransactionalDatatype,
        },
    };

    #[test]
    fn can_assert_send_and_sync_traits() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Counter>();
    }

    #[test]
    #[instrument]
    fn can_call_public_blanket_trait_methods() {
        let attr = new_attribute!(DataType::Counter);
        let key = attr.key.to_string();
        let transactional = TransactionalDatatype::new_arc(attr, Default::default());
        let counter = Counter::new(transactional);
        assert_eq!(counter.get_type(), DataType::Counter);
        assert_eq!(counter.get_key(), key);
        assert_eq!(counter.get_state(), Default::default());
    }

    #[test]
    #[instrument]
    fn can_use_counter_operations() {
        let counter = Counter::new(TransactionalDatatype::new_arc(
            new_attribute!(DataType::Counter),
            Default::default(),
        ));
        assert_eq!(1, counter.increase().unwrap());
        assert_eq!(11, counter.increase_by(10).unwrap());
        assert_eq!(11, counter.get_value());
    }

    #[test]
    #[instrument]
    fn can_use_transaction() {
        let counter = Counter::new(TransactionalDatatype::new_arc(
            new_attribute!(DataType::Counter),
            Default::default(),
        ));
        let result1 = counter.transaction("success", |c| {
            c.increase_by(1).unwrap();
            c.increase_by(2).unwrap();
            Ok(())
        });
        assert!(result1.is_ok());
        assert_eq!(3, counter.get_value());

        let result2 = counter.transaction("failure", |c| {
            c.increase_by(11).unwrap();
            c.increase_by(22).unwrap();
            Err("failed".into())
        });
        assert!(result2.is_err());
        assert_eq!(3, counter.get_value());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    #[instrument]
    async fn can_run_transactions_concurrently() {
        let counter = Counter::new(TransactionalDatatype::new_arc(
            new_attribute!(DataType::Counter),
            Default::default(),
        ));
        let mut join_handles = vec![];
        let parent_span = Span::current();

        for i in 0..5 {
            let counter = counter.clone();
            let parent_span = parent_span.clone();
            join_handles.push(tokio::spawn(async move {
                let thread_span = info_span!("run_transaction", i = i);
                thread_span.set_parent(parent_span.context()).unwrap();
                let _g1 = thread_span.enter();
                let tag = format!("tag:{i}");
                counter.transaction(tag, move |c| {
                    c.increase_by(i).unwrap();
                    Ok(())
                })
            }));
        }

        for jh in join_handles {
            let _ = jh.await.unwrap();
        }
        assert_eq!(1 + 2 + 3 + 4, counter.get_value());
    }
}
