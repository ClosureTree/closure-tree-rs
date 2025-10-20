use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait, Value,
};

use crate::config::AdvisoryLockStrategy;
use crate::error::ClosureTreeError;

pub struct LockedTransaction {
    txn: Option<DatabaseTransaction>,
    key: Option<String>,
}

impl LockedTransaction {
    pub async fn acquire(
        strategy: &AdvisoryLockStrategy,
        db: &DatabaseConnection,
    ) -> Result<Self, ClosureTreeError> {
        let key = match strategy {
            AdvisoryLockStrategy::Disabled => None,
            AdvisoryLockStrategy::Namespaced(key) => Some(key.as_str().to_owned()),
        };

        let txn = db.begin().await?;

        if let Some(ref key) = key {
            if let Err(err) = acquire_lock(&txn, key).await {
                let _ = txn.rollback().await;
                return Err(err);
            }
        }

        Ok(Self {
            txn: Some(txn),
            key,
        })
    }

    pub fn connection(&self) -> &DatabaseTransaction {
        self.txn.as_ref().expect("transaction already consumed")
    }

    pub async fn commit(mut self) -> Result<(), ClosureTreeError> {
        if let Some(ref key) = self.key {
            if let Some(txn) = self.txn.as_ref() {
                release_lock(txn, key).await?;
            }
        }

        if let Some(txn) = self.txn.take() {
            txn.commit().await?;
        }

        Ok(())
    }

    pub async fn rollback(mut self) -> Result<(), ClosureTreeError> {
        if let Some(ref key) = self.key {
            if let Some(txn) = self.txn.as_ref() {
                let _ = release_lock(txn, key).await;
            }
        }

        if let Some(txn) = self.txn.take() {
            txn.rollback().await?;
        }

        Ok(())
    }
}

async fn acquire_lock(txn: &DatabaseTransaction, key: &str) -> Result<(), ClosureTreeError> {
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_lock(hashtext($1), 0)",
        vec![Value::from(key)],
    ))
    .await?;
    Ok(())
}

async fn release_lock(txn: &DatabaseTransaction, key: &str) -> Result<(), ClosureTreeError> {
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_unlock(hashtext($1), 0)",
        vec![Value::from(key)],
    ))
    .await?;
    Ok(())
}
