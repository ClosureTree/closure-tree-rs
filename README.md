# closure-tree (Rust)

A SeaORM-friendly port of Ruby's [`closure_tree`](https://github.com/ClosureTree/closure_tree).

## Status

Early preview (`0.0.1`). PostgreSQL only. Contributions welcome.

## Quickstart

```toml
[dependencies]
closure-tree = "0.0.1"
```

```rust
use closure_tree::ClosureTreeRepository;
use closure_tree::ClosureTreeModelDerive as ClosureTreeModel;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, ClosureTreeModel)]
#[sea_orm(table_name = "nodes")]
#[closure_tree(hierarchy_module = "crate::entity::node_hierarchy", hierarchy_table = "node_hierarchies")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub parent_id: Option<i32>,
    pub name: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = sea_orm::Database::connect("postgres://...").await?;
    let repo = ClosureTreeRepository::<entity::node::Model>::new();

    let _leaf = repo
        .find_or_create_by_path(&db, &["root", "child", "leaf"])
        .await?;

    Ok(())
}
```

## Features

* Derive macro for SeaORM models (`#[derive(ClosureTreeModel)]`).
* Repository helpers (`parent`, `descendants`, `find_by_path`, `find_or_create_by_path`, etc.).
* Advisory locks via `pg_advisory_lock`, rebuild utilities.
* Integration test against a Docker Postgres instance.

## Limitations

* PostgreSQL only.
* Ordering, `hash_tree`, dependent strategies, and some Ruby APIs are not yet ported.
* Advisory lock is limited to Postgres advisory locks; no MySQL adapter yet.

## Development

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

Requires Docker Postgres at `postgres://closure_tree:closure_tree_pass@localhost:5434/closure_tree_test` (see `tests/postgres.rs`).
