use std::str::FromStr;

use crate::{domain::library::{LibraryType, ServerLibrary, ServerLibrarySettings}, model::{libraries::ServerLibraryForUpdate, store::SqliteStore}};
use super::Result;
use crate::domain::library::LibraryRole;
use rusqlite::{params, params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, ToSql};

impl FromSql for LibraryRole {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = LibraryRole::from_str(&as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}

impl ToSql for LibraryRole {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            LibraryRole::Admin  => "admin".to_sql(),
            LibraryRole::Read  => "read".to_sql(),
            LibraryRole::Write  => "write".to_sql(),
            LibraryRole::None  => "none".to_sql(),
        }
    }
}




// endregion: ---

impl FromSql for LibraryType {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {
            let r = LibraryType::from_str(&as_string).map_err(|_| FromSqlError::InvalidType);
            r
        })
    }
}


impl ToSql for LibraryType {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let l = &self.clone();
        let r = l.to_string();
        Ok(ToSqlOutput::from(r))
    }
}



// region:    --- Library Settings

impl FromSql for ServerLibrarySettings {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {

            let r = serde_json::from_str::<ServerLibrarySettings>(&as_string).map_err(|_| FromSqlError::InvalidType)?;

            Ok(r)
        })
    }
}

impl ToSql for ServerLibrarySettings {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let r = serde_json::to_string(&self).map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        Ok(ToSqlOutput::from(r))
    }
}
// endregion:    --- 


impl SqliteStore {
    // region:    --- Libraries
    pub async fn get_library(&self, library_id: &str) -> Result<Option<ServerLibrary>> {
        let library_id = library_id.to_string();
            let row = self.server_store.call( move |conn| { 
                let row = conn.query_row(
                "SELECT id, name, source, root, type, crypt, settings  FROM Libraries WHERE id = ?1",
                [&library_id],
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
                ).optional()?;
    
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

    pub async fn add_library(&self, library: ServerLibrary) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT INTO Libraries (id, name, type, source, root, settings, crypt)
            VALUES (?, ?, ? ,?, ?, ?, ?)", params![
                library.id,
                library.name,
                library.kind,
                library.source,
                library.root,
                library.settings,
                library.crypt
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn update_library(&self, library_id: &str, update: ServerLibraryForUpdate) -> Result<()> {
        let library_id = library_id.to_string();
        self.server_store.call( move |conn| { 
            let mut columns: Vec<&str> = Vec::new();
            let mut values: Vec<Box<dyn ToSql>> = Vec::new();
            if let Some(name) = update.name {
                columns.push("name = ?");
                values.push(Box::new(name));
            } 
            if let Some(source) = update.source {
                columns.push("source = ?");
                values.push(Box::new(source));
            } 
            if let Some(root) = update.root {
                columns.push("root = ?");
                values.push(Box::new(root));
            } 
            if let Some(settings) = update.settings {
                columns.push("settings = ?");
                values.push(Box::new(settings));
            } 
            if columns.len() > 0 {
                values.push(Box::new(library_id));
                let update_sql = format!("UPDATE Libraries SET {} WHERE id = ?", columns.join(", "));
                conn.execute(&update_sql, params_from_iter(values))?;
            }
            Ok(())
        }).await?;
        Ok(())
    }
    
    
    // endregion:    --- Libraries
    
    }