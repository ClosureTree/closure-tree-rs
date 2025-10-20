use closure_tree::ClosureTreeRepository;
use sea_orm::entity::prelude::*;
use sea_orm::{Database, DatabaseConnection, DbBackend, Statement};

mod entity {
    pub mod node {
        use closure_tree::ClosureTreeModelDerive as ClosureTreeModel;
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel, ClosureTreeModel)]
        #[sea_orm(table_name = "nodes")]
        #[closure_tree(
            hierarchy_module = "crate::entity::node_hierarchy",
            hierarchy_table = "node_hierarchies"
        )]
        pub struct Model {
            #[sea_orm(primary_key)]
            pub id: i32,
            pub parent_id: Option<i32>,
            pub name: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    pub mod node_hierarchy {
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "node_hierarchies")]
        pub struct Model {
            #[sea_orm(primary_key)]
            pub ancestor_id: i32,
            #[sea_orm(primary_key)]
            pub descendant_id: i32,
            pub generations: i32,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }
}

#[tokio::test]
async fn find_or_create_path_builds_hierarchy() -> Result<(), Box<dyn std::error::Error>> {
    let db = setup_database().await?;
    truncate_tables(&db).await?;

    let repo = ClosureTreeRepository::<entity::node::Model>::new();

    let leaf = repo
        .find_or_create_by_path(&db, &["root", "child", "leaf"])
        .await?;

    assert_eq!(leaf.name, "leaf");

    let child = repo
        .find_by_path(&db, &["root", "child"])
        .await?
        .expect("child node exists");

    let descendants = repo.descendants(&db, &child).await?;
    let names: Vec<String> = descendants.into_iter().map(|node| node.name).collect();
    assert_eq!(names, vec!["leaf"]);

    Ok(())
}

async fn setup_database() -> Result<DatabaseConnection, sea_orm::DbErr> {
    let url = std::env::var("CLOSURE_TREE_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgres://closure_tree:closure_tree_pass@localhost:5434/closure_tree_test".to_string()
        });

    Database::connect(url).await
}

async fn truncate_tables(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_string(
        DbBackend::Postgres,
        r#"
        CREATE TABLE IF NOT EXISTS nodes (
            id SERIAL PRIMARY KEY,
            parent_id INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
            name TEXT NOT NULL
        );
        "#,
    ))
    .await?;

    db.execute(Statement::from_string(
        DbBackend::Postgres,
        r#"
        CREATE TABLE IF NOT EXISTS node_hierarchies (
            ancestor_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            descendant_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            generations INTEGER NOT NULL,
            PRIMARY KEY (ancestor_id, descendant_id)
        );
        "#,
    ))
    .await?;

    db.execute(Statement::from_string(
        DbBackend::Postgres,
        "TRUNCATE TABLE node_hierarchies RESTART IDENTITY CASCADE;",
    ))
    .await?;

    db.execute(Statement::from_string(
        DbBackend::Postgres,
        "TRUNCATE TABLE nodes RESTART IDENTITY CASCADE;",
    ))
    .await?;

    Ok(())
}
