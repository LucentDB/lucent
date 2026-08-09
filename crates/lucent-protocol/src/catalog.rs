//! Normalized catalog types.
//!
//! Driver-neutral by construction: a Postgres schema, a DuckDB catalog+schema
//! pair, and a BigQuery project+dataset all render as a `Vec<String>` path.
//! Drivers answer these requests however they like — the app never sees
//! provider SQL, which is the entire point of this module.
//!
//! Counts are `Option`: `None` means "this driver cannot tell you cheaply",
//! which callers MUST render as unknown rather than as zero.

use serde::{Deserialize, Serialize};

/// A namespace path, most-general segment first.
///
/// Postgres: `["public"]`. DuckDB: `["memory", "main"]`.
/// BigQuery: `["my-project", "my_dataset"]`.
pub type NamespacePath = Vec<String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    pub path: NamespacePath,
    /// Objects in this namespace when the driver can count them cheaply.
    pub object_count: Option<u64>,
}

impl Namespace {
    /// Dotted rendering. For a single-segment path this is just the segment,
    /// which is what the existing frontend contract expects.
    pub fn display(&self) -> String {
        self.path.join(".")
    }
}

/// What a catalog object is. `Other` is the escape hatch that stops this enum
/// from having to model every provider's object taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Function,
    Sequence,
    Other(String),
}

impl ObjectKind {
    /// Stable lowercase name. These strings are a contract with the sidebar
    /// (`SchemaObject.kind`) and with the AI tool schema. Do not change them.
    pub fn as_str(&self) -> &str {
        match self {
            ObjectKind::Table => "table",
            ObjectKind::View => "view",
            ObjectKind::MaterializedView => "matview",
            ObjectKind::Function => "function",
            ObjectKind::Sequence => "sequence",
            ObjectKind::Other(s) => s.as_str(),
        }
    }

    /// Inverse of `as_str`. Unknown names degrade to `Other` — a catalog listing
    /// must never fail because a driver grew a new object type.
    pub fn from_label(s: &str) -> Self {
        match s {
            "table" => ObjectKind::Table,
            "view" => ObjectKind::View,
            "matview" => ObjectKind::MaterializedView,
            "function" => ObjectKind::Function,
            "sequence" => ObjectKind::Sequence,
            other => ObjectKind::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectRef {
    pub namespace: NamespacePath,
    pub name: String,
    pub kind: ObjectKind,
}

/// Partitioning metadata for a partitioned parent.
///
/// Partitioning is a general warehouse concept, not a Postgres quirk, so it
/// belongs in the normalized model. `ai/schema_graph.rs` collapses partition
/// children into their parent deliberately — indexing 84 near-identical
/// partitions poisons retrieval — and needs this to keep doing so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionInfo {
    /// Driver-rendered key description, e.g. `"RANGE (created_at)"`.
    pub key: Option<String>,
    pub child_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectSummary {
    pub reference: ObjectRef,
    /// Estimated row count. `None` when the driver has no cheap estimate, or
    /// when the estimate has never been computed. Never conflate with `Some(0)`.
    pub est_rows: Option<u64>,
    pub comment: Option<String>,
    /// Present only for partitioned parents.
    pub partition: Option<PartitionInfo>,
    /// True when a parent already covers this object. Callers that index a
    /// schema for retrieval should skip these.
    pub is_partition_child: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeyTarget {
    pub namespace: NamespacePath,
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDetail {
    pub name: String,
    /// Driver-rendered type name, as precise as the driver can make it.
    pub type_name: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    /// 1-based position within the object.
    pub ordinal: u32,
    pub default: Option<String>,
    pub comment: Option<String>,
    /// Set when this column is the referencing side of a foreign key.
    pub foreign_key: Option<ForeignKeyTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDetail {
    pub reference: ObjectRef,
    pub columns: Vec<ColumnDetail>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPath {
    pub namespace: NamespacePath,
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    pub from: ColumnPath,
    pub to: ColumnPath,
}

/// One hit from a name search. `column` is `Some` when the match was on a
/// column name rather than the object name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub reference: ObjectRef,
    pub column: Option<String>,
    pub score: f32,
}

/// A driver-defined key/value property (sequence increment, table format, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectProperty {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CatalogRequest {
    /// Every user-visible namespace.
    ListNamespaces,
    /// Objects in one namespace. Empty `kinds` means every kind.
    ListObjects {
        namespace: NamespacePath,
        kinds: Vec<ObjectKind>,
    },
    /// Every object in the connection, across namespaces. This exists so the
    /// schema cache and the AI index do not have to issue one request per
    /// namespace — that N+1 is what this whole plan removes.
    ListAllObjects {
        kinds: Vec<ObjectKind>,
    },
    /// Columns, keys, comments for specific objects. Batched by construction.
    DescribeObjects {
        refs: Vec<ObjectRef>,
    },
    /// Every foreign key in the connection.
    ListForeignKeys,
    /// Name search over objects and columns. Ranking is driver-specific
    /// (Postgres uses pg_trgm when installed), which is why it lives here
    /// rather than being reconstructed from `ListAllObjects` in the app.
    SearchObjects {
        query: String,
        kinds: Vec<ObjectKind>,
        namespace: Option<NamespacePath>,
        limit: u32,
    },
    GetObjectDdl {
        reference: ObjectRef,
    },
    GetObjectProperties {
        reference: ObjectRef,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CatalogResult {
    Namespaces(Vec<Namespace>),
    Objects(Vec<ObjectSummary>),
    ObjectDetails(Vec<ObjectDetail>),
    ForeignKeys(Vec<ForeignKey>),
    SearchHits(Vec<SearchHit>),
    Ddl(String),
    Properties(Vec<ObjectProperty>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_ref() -> ObjectRef {
        ObjectRef {
            namespace: vec!["public".into()],
            name: "users".into(),
            kind: ObjectKind::Table,
        }
    }

    #[test]
    fn object_kind_strings_are_the_frontend_contract() {
        // These exact strings reach `SchemaObject.kind` in the sidebar and the
        // AI tool schema. Changing one silently breaks both.
        assert_eq!(ObjectKind::Table.as_str(), "table");
        assert_eq!(ObjectKind::View.as_str(), "view");
        assert_eq!(ObjectKind::MaterializedView.as_str(), "matview");
        assert_eq!(ObjectKind::Function.as_str(), "function");
        assert_eq!(ObjectKind::Sequence.as_str(), "sequence");
        assert_eq!(ObjectKind::Other("domain".into()).as_str(), "domain");
    }

    #[test]
    fn object_kind_round_trips_through_its_string_form() {
        for kind in [
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::MaterializedView,
            ObjectKind::Function,
            ObjectKind::Sequence,
        ] {
            assert_eq!(ObjectKind::from_label(kind.as_str()), kind);
        }
        // Unknown names take the escape hatch rather than erroring.
        assert_eq!(
            ObjectKind::from_label("aggregate"),
            ObjectKind::Other("aggregate".into())
        );
    }

    #[test]
    fn a_single_segment_namespace_renders_as_todays_schema_name() {
        // The frontend contract is a flat schema string. Postgres emits one
        // segment, so joining must reproduce it exactly.
        let ns = Namespace {
            path: vec!["public".into()],
            object_count: Some(12),
        };
        assert_eq!(ns.display(), "public");

        // Multi-segment (DuckDB catalog.schema, BigQuery project.dataset)
        // renders dotted — this is what makes the type driver-neutral.
        let ns = Namespace {
            path: vec!["memory".into(), "main".into()],
            object_count: None,
        };
        assert_eq!(ns.display(), "memory.main");
    }

    #[test]
    fn unknown_counts_are_none_not_zero() {
        // A never-analyzed table must be distinguishable from an empty one.
        let unknown = ObjectSummary {
            reference: table_ref(),
            est_rows: None,
            comment: None,
            partition: None,
            is_partition_child: false,
        };
        let empty = ObjectSummary {
            est_rows: Some(0),
            ..unknown.clone()
        };
        assert_ne!(unknown.est_rows, empty.est_rows);
    }

    #[test]
    fn round_trips_every_catalog_request_through_bincode() {
        let requests = vec![
            CatalogRequest::ListNamespaces,
            CatalogRequest::ListObjects {
                namespace: vec!["public".into()],
                kinds: vec![ObjectKind::Table, ObjectKind::View],
            },
            CatalogRequest::ListAllObjects {
                kinds: vec![ObjectKind::Table],
            },
            CatalogRequest::DescribeObjects {
                refs: vec![table_ref()],
            },
            CatalogRequest::ListForeignKeys,
            CatalogRequest::SearchObjects {
                query: "user".into(),
                kinds: vec![],
                namespace: None,
                limit: 10,
            },
            CatalogRequest::GetObjectDdl {
                reference: table_ref(),
            },
            CatalogRequest::GetObjectProperties {
                reference: table_ref(),
            },
        ];
        for r in requests {
            let bytes = bincode::serialize(&r).expect("serialize");
            let back: CatalogRequest = bincode::deserialize(&bytes).expect("deserialize");
            assert_eq!(format!("{r:?}"), format!("{back:?}"));
        }
    }

    #[test]
    fn round_trips_every_catalog_result_through_bincode() {
        let results = vec![
            CatalogResult::Namespaces(vec![Namespace {
                path: vec!["public".into()],
                object_count: Some(3),
            }]),
            CatalogResult::Objects(vec![ObjectSummary {
                reference: table_ref(),
                est_rows: Some(42),
                comment: Some("people".into()),
                partition: Some(PartitionInfo {
                    key: Some("RANGE (created_at)".into()),
                    child_count: 84,
                }),
                is_partition_child: false,
            }]),
            CatalogResult::ObjectDetails(vec![ObjectDetail {
                reference: table_ref(),
                comment: None,
                columns: vec![ColumnDetail {
                    name: "id".into(),
                    type_name: "int8".into(),
                    nullable: false,
                    is_primary_key: true,
                    ordinal: 1,
                    default: Some("nextval('users_id_seq'::regclass)".into()),
                    comment: None,
                    foreign_key: None,
                }],
            }]),
            CatalogResult::ForeignKeys(vec![ForeignKey {
                from: ColumnPath {
                    namespace: vec!["public".into()],
                    table: "orders".into(),
                    column: "user_id".into(),
                },
                to: ColumnPath {
                    namespace: vec!["public".into()],
                    table: "users".into(),
                    column: "id".into(),
                },
            }]),
            CatalogResult::SearchHits(vec![SearchHit {
                reference: table_ref(),
                column: Some("user_id".into()),
                score: 0.75,
            }]),
            CatalogResult::Ddl("CREATE VIEW ...".into()),
            CatalogResult::Properties(vec![ObjectProperty {
                key: "Increment".into(),
                value: "1".into(),
            }]),
        ];
        for r in results {
            let bytes = bincode::serialize(&r).expect("serialize");
            let back: CatalogResult = bincode::deserialize(&bytes).expect("deserialize");
            assert_eq!(format!("{r:?}"), format!("{back:?}"));
        }
    }
}
