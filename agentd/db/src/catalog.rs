//! Product migration catalog (D45.8): files live in `agentd/migrations/`.

use crate::Migration;

/// Known migrations in ascending version order. The source of truth is `agentd/migrations/` (D45.8).
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../migrations/0001_init.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 2,
        name: "bar",
        sql: include_str!("../../migrations/0002_bar.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 3,
        name: "sessions",
        sql: include_str!("../../migrations/0003_sessions.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 4,
        name: "chat",
        sql: include_str!("../../migrations/0004_chat.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 5,
        name: "events_gateway",
        sql: include_str!("../../migrations/0005_events_gateway.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 6,
        name: "tasks",
        sql: include_str!("../../migrations/0006_tasks.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 7,
        name: "modules_tunnels",
        sql: include_str!("../../migrations/0007_modules_tunnels.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 8,
        name: "memory_facts",
        sql: include_str!("../../migrations/0008_memory_facts.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
    Migration {
        version: 9,
        name: "web_token",
        sql: include_str!("../../migrations/0009_web_token.sql"),
        // `api_token` recreation: data moves unchanged, but a snapshot before the migration is not superfluous.
        mutates_data: true,
        recreates_tables: true,
    },
    Migration {
        version: 10,
        name: "permissions_mcp",
        sql: include_str!("../../migrations/0010_permissions_mcp.sql"),
        mutates_data: false,
        recreates_tables: false,
    },
];
