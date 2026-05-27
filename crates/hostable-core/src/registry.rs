use crate::Result;
use crate::adapter::{GameKind, PortSpec};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerId(pub String);

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRecord {
    pub id: ServerId,
    pub name: String,
    pub game: GameKind,
    pub install_path: PathBuf,
    pub service_name: String,
    pub ports: Vec<PortSpec>,
    pub adapter_config: serde_json::Value,
}

#[derive(Clone)]
pub struct RegistryStore {
    conn: Arc<Mutex<Connection>>,
}

impl RegistryStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS servers (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                game TEXT NOT NULL,
                install_path TEXT NOT NULL,
                service_name TEXT NOT NULL,
                ports_json TEXT NOT NULL,
                adapter_config_json TEXT NOT NULL
            );
            "#,
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn list_servers(&self) -> Result<Vec<ServerRecord>> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, game, install_path, service_name, ports_json, adapter_config_json
            FROM servers
            ORDER BY name COLLATE NOCASE
            "#,
        )?;

        let rows = stmt.query_map([], row_to_server_record)?;
        let mut servers = Vec::new();
        for row in rows {
            servers.push(row?);
        }
        Ok(servers)
    }

    pub fn get_server(&self, id: &ServerId) -> Result<Option<ServerRecord>> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        conn.query_row(
            r#"
            SELECT id, name, game, install_path, service_name, ports_json, adapter_config_json
            FROM servers
            WHERE id = ?1
            "#,
            params![id.0],
            row_to_server_record,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_server(&self, server: &ServerRecord) -> Result<()> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO servers (
                id, name, game, install_path, service_name, ports_json, adapter_config_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                game = excluded.game,
                install_path = excluded.install_path,
                service_name = excluded.service_name,
                ports_json = excluded.ports_json,
                adapter_config_json = excluded.adapter_config_json
            "#,
            params![
                server.id.0,
                server.name,
                serde_json::to_string(&server.game)?,
                server.install_path.to_string_lossy(),
                server.service_name,
                serde_json::to_string(&server.ports)?,
                serde_json::to_string(&server.adapter_config)?,
            ],
        )?;
        Ok(())
    }
}

fn row_to_server_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerRecord> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let game_json: String = row.get(2)?;
    let install_path: String = row.get(3)?;
    let service_name: String = row.get(4)?;
    let ports_json: String = row.get(5)?;
    let adapter_config_json: String = row.get(6)?;

    let game = serde_json::from_str::<GameKind>(&game_json).map_err(json_to_sql_error)?;
    let ports = serde_json::from_str::<Vec<PortSpec>>(&ports_json).map_err(json_to_sql_error)?;
    let adapter_config = serde_json::from_str::<serde_json::Value>(&adapter_config_json)
        .map_err(json_to_sql_error)?;

    Ok(ServerRecord {
        id: ServerId(id),
        name,
        game,
        install_path: PathBuf::from(install_path),
        service_name,
        ports,
        adapter_config,
    })
}

fn json_to_sql_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{Protocol, ServerType};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn stores_and_loads_server_records() {
        let path = std::env::temp_dir().join(format!(
            "hostable-registry-{}.sqlite",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = RegistryStore::open(&path).unwrap();
        let record = ServerRecord {
            id: ServerId("friends".to_string()),
            name: "Friends".to_string(),
            game: GameKind::Minecraft,
            install_path: "/srv/hostable/servers/friends".into(),
            service_name: "hostable-friends.service".to_string(),
            ports: vec![PortSpec {
                port: 25565,
                protocol: Protocol::Tcp,
                label: "Minecraft server".to_string(),
            }],
            adapter_config: serde_json::json!({
                "server_type": ServerType::Fabric
            }),
        };

        store.upsert_server(&record).unwrap();

        let loaded = store.get_server(&record.id).unwrap().unwrap();
        assert_eq!(loaded, record);
        assert_eq!(store.list_servers().unwrap().len(), 1);
    }
}
