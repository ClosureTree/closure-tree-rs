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
            "postgres://closure_tree:closure_tree_pass@localhost:5432/closure_tree_test".to_string()
        });

    Database::connect(url).await
}

async fn truncate_tables(db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    // Drop and recreate tables to avoid sequence conflicts
    db.execute(Statement::from_string(
        DbBackend::Postgres,
        "DROP TABLE IF EXISTS node_hierarchies CASCADE;",
    ))
    .await?;

    db.execute(Statement::from_string(
        DbBackend::Postgres,
        "DROP TABLE IF EXISTS nodes CASCADE;",
    ))
    .await?;

    db.execute(Statement::from_string(
        DbBackend::Postgres,
        r#"
        CREATE TABLE nodes (
            id SERIAL PRIMARY KEY,
            parent_id INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
            name TEXT NOT NULL UNIQUE
        );
        "#,
    ))
    .await?;

    db.execute(Statement::from_string(
        DbBackend::Postgres,
        r#"
        CREATE TABLE node_hierarchies (
            ancestor_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            descendant_id INTEGER NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
            generations INTEGER NOT NULL,
            PRIMARY KEY (ancestor_id, descendant_id)
        );
        "#,
    ))
    .await?;

    Ok(())
}

#[tokio::test]
async fn bulk_insert_with_in_batch_parents() -> Result<(), Box<dyn std::error::Error>> {
    use closure_tree::{BulkInsertOptions, InsertNode, ParentRef};
    use entity::node::ActiveModel;
    use sea_orm::ActiveValue;

    let db = setup_database().await?;
    truncate_tables(&db).await?;

    let repo = ClosureTreeRepository::<entity::node::Model>::new();

    // Create bulk insert nodes with InBatch parent references
    let nodes = vec![
        // Wave 0: roots
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("root1".to_string()),
            },
            parent_ref: None,
        },
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("root2".to_string()),
            },
            parent_ref: None,
        },
        // Wave 1: children of roots
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("child1_1".to_string()),
            },
            parent_ref: Some(ParentRef::InBatch(0)), // parent is root1
        },
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("child1_2".to_string()),
            },
            parent_ref: Some(ParentRef::InBatch(0)), // parent is root1
        },
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("child2_1".to_string()),
            },
            parent_ref: Some(ParentRef::InBatch(1)), // parent is root2
        },
        // Wave 2: grandchildren
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("grandchild1_1_1".to_string()),
            },
            parent_ref: Some(ParentRef::InBatch(2)), // parent is child1_1
        },
    ];

    let options = BulkInsertOptions::new("name"); // Use name as conflict column for testing

    let result = repo.bulk_insert_with_parent_ids(&db, nodes, options).await?;

    // Verify result
    assert_eq!(result.inserted, 6);
    assert_eq!(result.skipped, 0);
    assert_eq!(result.models.len(), 6);

    // Verify hierarchy was built correctly
    let root1 = repo
        .find_by_path(&db, &["root1"])
        .await?
        .expect("root1 exists");

    let descendants = repo.descendants(&db, &root1).await?;
    let names: Vec<String> = descendants.into_iter().map(|node| node.name).collect();

    // root1 should have child1_1, child1_2, and grandchild1_1_1 as descendants
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"child1_1".to_string()));
    assert!(names.contains(&"child1_2".to_string()));
    assert!(names.contains(&"grandchild1_1_1".to_string()));

    Ok(())
}

#[tokio::test]
async fn bulk_insert_detects_cycles() -> Result<(), Box<dyn std::error::Error>> {
    use closure_tree::{BulkInsertOptions, ClosureTreeError, InsertNode, ParentRef};
    use entity::node::ActiveModel;
    use sea_orm::ActiveValue;

    let db = setup_database().await?;
    truncate_tables(&db).await?;

    let repo = ClosureTreeRepository::<entity::node::Model>::new();

    // Create nodes with circular dependency
    let nodes = vec![
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("node1".to_string()),
            },
            parent_ref: Some(ParentRef::InBatch(1)), // depends on node2
        },
        InsertNode {
            model: ActiveModel {
                id: ActiveValue::NotSet,
                parent_id: ActiveValue::NotSet,
                name: ActiveValue::Set("node2".to_string()),
            },
            parent_ref: Some(ParentRef::InBatch(0)), // depends on node1
        },
    ];

    let options = BulkInsertOptions::new("name");
    let result = repo.bulk_insert_with_parent_ids(&db, nodes, options).await;

    // Should detect the cycle
    assert!(result.is_err());
    match result {
        Err(ClosureTreeError::CycleDetected(_)) => {}
        _ => panic!("Expected CycleDetected error"),
    }

    Ok(())
}
