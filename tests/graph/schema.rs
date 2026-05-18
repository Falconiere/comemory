use qwick::graph::schema::{CODE_LAYER_DDL, MEMORY_LAYER_DDL};

#[test]
fn memory_layer_ddl_is_non_empty() {
    assert!(
        !MEMORY_LAYER_DDL.is_empty(),
        "MEMORY_LAYER_DDL must contain at least one statement",
    );
}

#[test]
fn memory_layer_ddl_uses_if_not_exists_everywhere() {
    for ddl in MEMORY_LAYER_DDL {
        assert!(
            ddl.contains("IF NOT EXISTS"),
            "every DDL statement must be idempotent; offender: {ddl}",
        );
    }
}

#[test]
fn memory_layer_ddl_defines_required_node_tables() {
    let joined = MEMORY_LAYER_DDL.join("\n");
    for table in ["Memory", "Repo", "Author", "Tag"] {
        let needle = format!("CREATE NODE TABLE IF NOT EXISTS {table}");
        assert!(
            joined.contains(&needle),
            "expected NODE TABLE for {table} in DDL, got:\n{joined}",
        );
    }
}

#[test]
fn memory_layer_ddl_defines_required_rel_tables() {
    let joined = MEMORY_LAYER_DDL.join("\n");
    for rel in [
        "InRepo",
        "AuthoredBy",
        "Tagged",
        "Supersedes",
        "ConflictsWith",
        "RelatesTo",
        "DerivedFrom",
    ] {
        let needle = format!("CREATE REL TABLE IF NOT EXISTS {rel}");
        assert!(
            joined.contains(&needle),
            "expected REL TABLE for {rel} in DDL, got:\n{joined}",
        );
    }
}

#[test]
fn memory_node_table_has_primary_key_on_id() {
    let memory_ddl = MEMORY_LAYER_DDL
        .iter()
        .find(|s| s.contains("NODE TABLE IF NOT EXISTS Memory"))
        .expect("Memory node table DDL must exist");
    assert!(
        memory_ddl.contains("PRIMARY KEY(id)"),
        "Memory must be keyed on id, got: {memory_ddl}",
    );
}

#[test]
fn code_layer_ddl_has_seven_statements() {
    assert_eq!(
        CODE_LAYER_DDL.len(),
        7,
        "code layer expects 2 node tables + 5 rel tables = 7 statements",
    );
}

#[test]
fn code_layer_ddl_uses_if_not_exists_everywhere() {
    for ddl in CODE_LAYER_DDL {
        assert!(
            ddl.contains("IF NOT EXISTS"),
            "every code-layer DDL must be idempotent; offender: {ddl}",
        );
    }
}

#[test]
fn code_layer_ddl_defines_required_node_tables() {
    let joined = CODE_LAYER_DDL.join("\n");
    for table in ["File", "Symbol"] {
        let needle = format!("CREATE NODE TABLE IF NOT EXISTS {table}");
        assert!(
            joined.contains(&needle),
            "expected NODE TABLE for {table} in code DDL, got:\n{joined}",
        );
    }
}

#[test]
fn code_layer_ddl_defines_required_rel_tables() {
    let joined = CODE_LAYER_DDL.join("\n");
    for rel in [
        "DefinedIn",
        "Calls",
        "Imports",
        "ReferencesFile",
        "ReferencesSymbol",
    ] {
        let needle = format!("CREATE REL TABLE IF NOT EXISTS {rel}");
        assert!(
            joined.contains(&needle),
            "expected REL TABLE for {rel} in code DDL, got:\n{joined}",
        );
    }
}

#[test]
fn file_node_table_has_primary_key_on_qualified() {
    let ddl = CODE_LAYER_DDL
        .iter()
        .find(|s| s.contains("NODE TABLE IF NOT EXISTS File"))
        .expect("File node table DDL must exist");
    assert!(
        ddl.contains("PRIMARY KEY(qualified)"),
        "File must be keyed on qualified, got: {ddl}",
    );
}

#[test]
fn symbol_node_table_has_primary_key_on_qualified() {
    let ddl = CODE_LAYER_DDL
        .iter()
        .find(|s| s.contains("NODE TABLE IF NOT EXISTS Symbol"))
        .expect("Symbol node table DDL must exist");
    assert!(
        ddl.contains("PRIMARY KEY(qualified)"),
        "Symbol must be keyed on qualified, got: {ddl}",
    );
}
