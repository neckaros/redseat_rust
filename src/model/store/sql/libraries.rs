use crate::{domain::library::ServerLibrary, model::store::SqliteStore};
use super::Result;

impl SqliteStore {
    // region:    --- Libraries
    pub async fn get_library(&self, user_id: &str) -> Result<ServerLibrary> {
        let user_id = user_id.to_string();
            let row = self.server_store.call( move |conn| { 
                let row = conn.query_row(
                "SELECT id, name, source, root, type, crypt, settings  FROM Libraries WHERE id = ?1",
                [&user_id],
                |row| {
                    Ok(ServerLibrary {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        source:  row.get(2)?,
                        root:  row.get(3)?,
                        kind:  row.get(4)?,
                        crypt:  row.get(5)?,
                        settings:  row.get(6)?,
                    })
                },
                )?;
    
                Ok(row)
        }).await?;
        Ok(row)
    }
    
    pub async fn get_libraries(&self) -> Result<Vec<ServerLibrary>> {
        let row = self.server_store.call( move |conn| { 
            let mut query = conn.prepare("SELECT id, name, source, root, type, crypt, settings  FROM Libraries")?;
            let rows = query.query_map(
            [],
            |row| {
                Ok(ServerLibrary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source:  row.get(2)?,
                    root:  row.get(3)?,
                    kind:  row.get(4)?,
                    crypt:  row.get(5)?,
                    settings:  row.get(6)?,
                })
            },
            )?;
            let libraries:Vec<ServerLibrary> = rows.collect::<std::result::Result<Vec<ServerLibrary>, rusqlite::Error>>()?; 
            Ok(libraries)
        }).await?;
        Ok(row)
    }
    
    // endregion:    --- Libraries
    
    }