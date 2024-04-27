use std::str::FromStr;

use crate::{domain::plugin::{Plugin, PluginForInsert, PluginForUpdate, PluginSettings}, model::{error::Error, plugins::PluginQuery, store::{from_comma_separated, to_comma_separated, to_comma_separated_optional, SqliteStore}}, tools::array_tools::replace_add_remove_from_array};

use super::{QueryBuilder, QueryWhereType, Result};
use rusqlite::{params, params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, OptionalExtension, Row, ToSql};

use rs_plugin_common_interfaces::PluginType;


// endregion: ---



// region:    --- plugin Settings

impl FromSql for PluginSettings {
    fn column_result(value: ValueRef) -> FromSqlResult<Self> {
        String::column_result(value).and_then(|as_string| {

            let r = serde_json::from_str::<PluginSettings>(&as_string).map_err(|_| FromSqlError::InvalidType)?;

            Ok(r)
        })
    }
}

impl ToSql for PluginSettings {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        let r = serde_json::to_string(&self).map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        Ok(ToSqlOutput::from(r))
    }
}
// endregion:    --- 


impl SqliteStore {

    fn row_to_plugin(row: &Row) -> rusqlite::Result<Plugin> { 
        Ok(Plugin {
            id: row.get(0)?,
            name: row.get(1)?,
            path:  row.get(2)?,
            capabilities:  from_comma_separated(row.get(3)?),
            settings:  row.get(4)?,
            libraries: from_comma_separated(row.get(5)?),
            credential:  row.get(6)?,
            credential_type:  row.get(7)?,
            description:  row.get(8)?,
            version:  row.get(9)?,
            installed: true,
            ..Default::default()
        })
    }
    pub async fn get_plugin(&self, plugin_id: &str) -> Result<Option<Plugin>> {
        let plugin_id = plugin_id.to_string();
            let row = self.server_store.call( move |conn| { 
                let row = conn.query_row(
                "SELECT id, name, path, kind, settings, libraries, credential, credtype, desc, version FROM plugins WHERE id = ?1",
                [&plugin_id],
                Self::row_to_plugin,
                ).optional()?;
    
                Ok(row)
        }).await?;
        Ok(row)
    }
    
    pub async fn get_plugins(&self, query: PluginQuery) -> Result<Vec<Plugin>> {
        let row = self.server_store.call( move |conn| { 

            let mut where_query = QueryBuilder::new();
            if let Some(q) = &query.kind {
                where_query.add_where(super::QueryWhereType::SeparatedContain("kind", ",".to_string(), q));
            }
            if let Some(q) = &query.library {
                where_query.add_where(super::QueryWhereType::SeparatedContain("libraries", ",".to_string(), q));
            }

            let mut query = conn.prepare(&format!("SELECT id, name, path, kind, settings, libraries, credential, credtype, desc, version FROM plugins 
            {}", where_query.format()))?;
            //println!("query {:?}", query.expanded_sql());
            let rows = query.query_map(
            where_query.values(),
            Self::row_to_plugin,
            )?;
            let libraries:Vec<Plugin> = rows.collect::<std::result::Result<Vec<Plugin>, rusqlite::Error>>()?; 
            Ok(libraries)
        }).await?;
        Ok(row)
    }

    pub async fn remove_plugin(&self, plugin_id: String) -> Result<()> {
        self.server_store.call( move |conn| { 
            conn.execute("DELETE FROM plugins WHERE id = ?", [&plugin_id])?;
            Ok(())
        }).await?;
        Ok(())
    }

    pub async fn add_plugin(&self, plugin: PluginForInsert) -> Result<()> {
        self.server_store.call( move |conn| { 

            conn.execute("INSERT INTO plugins (id, name, path, kind, settings, libraries, credential, credtype, desc, version)
            VALUES (?, ?, ? ,?, ?, ?, ?, ?, ?, ?)", params![
                plugin.id,
                plugin.plugin.name,
                plugin.plugin.path,
                to_comma_separated(plugin.plugin.capabilities),
                plugin.plugin.settings,
                to_comma_separated(plugin.plugin.libraries),
                plugin.plugin.credential,
                plugin.plugin.credential_type,
                plugin.plugin.description,
                plugin.plugin.version
            ])?;
            
            Ok(())
        }).await?;
        Ok(())
    }
    
    pub async fn update_plugin(&self, plugin_id: &str, update: PluginForUpdate) -> Result<()> {
        let plugin_id = plugin_id.to_string();
        let existing = self.get_plugin(&plugin_id).await?.ok_or_else( || Error::NotFound)?;
        self.server_store.call( move |conn| { 
            let mut where_query = QueryBuilder::new();
            
            
            where_query.add_update(&update.name, "name");
            where_query.add_update(&update.path, "path");
            where_query.add_update(&update.description, "desc");
            where_query.add_update(&update.version, "version");
            where_query.add_update(&update.credential_type, "credtype");
            where_query.add_update(&update.settings, "settings");
            where_query.add_update(&update.credential, "credential");
            
            let capa = to_comma_separated_optional(update.capabilities);
            where_query.add_update(&capa, "kind");

            where_query.add_update(&update.credential, "credential");

            if update.remove_credential {
                where_query.add_nullify("credential");
            }

            //println!("{:?}", update);
            let libraries = replace_add_remove_from_array(Some(existing.libraries.clone()), update.libraries, update.add_libraries, update.remove_libraries);
            let v = to_comma_separated_optional(libraries);
            where_query.add_update(&v, "libraries");

            where_query.add_where(QueryWhereType::Equal("id", &plugin_id));
            if !where_query.columns_update.is_empty() {
                let update_sql = format!("UPDATE plugins SET {} {}", where_query.format_update(), where_query.format());
                conn.execute(&update_sql, where_query.values())?;
            }
            Ok(())
        }).await?;
        Ok(())
    }

    
    }