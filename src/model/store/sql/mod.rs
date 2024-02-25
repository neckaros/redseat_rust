pub mod libraries;
pub mod users;
pub mod credentials;
pub mod backups;
pub mod library;

use rusqlite::{params_from_iter, types::FromSql, ParamsFromIter, ToSql};
use tokio_rusqlite::Connection;

use crate::tools::log::{log_info, LogServiceType};

use super::{Result, SqliteStore};


pub async fn migrate_database(connection: &Connection) -> Result<usize> {
    let version = connection.call( |conn| {
        let version = conn.query_row(
            "SELECT user_version FROM pragma_user_version;",
            [],
            |row| {
                let version: usize = row.get(0)?;
                Ok(version)
            })?;

            if version < 2 {
                let initial = String::from_utf8_lossy(include_bytes!("001 - INITIAL.sql"));
                conn.execute_batch(&initial)?;
                
                conn.pragma_update(None, "user_version", 2)?;
                println!("Update SQL to verison 2")
            }
            
            Ok(version)
    }).await?;

    Ok(version)
} 


pub fn add_for_sql_update<'a, T: ToSql + 'a,>(optional: Option<T>, name: &str, columns: &mut Vec<String>, values: &mut Vec<Box<dyn ToSql + 'a>>) {
    if let Some(value) = optional {
        let r = format!("{} = ?", name.to_string());
        columns.push(r);
        values.push(Box::new(value));
    } 
}



pub enum QueryWhereType {
    Like(String),
    Equal(String),
    Custom(String),
}
pub struct QueryBuilder<'a> {
    columns_where: Vec<String>,
    values_where: Vec<Box<dyn ToSql + 'a>>,
    columns_update: Vec<String>,
    values_update: Vec<Box<dyn ToSql + 'a>>
}

impl <'a> QueryBuilder<'a> {
    pub fn new() -> Self {
        Self {
            columns_where: Vec::new(),
            values_where: Vec::new(),
            columns_update: Vec::new(),
            values_update: Vec::new()
        }
    }

    pub fn add_update<T: ToSql + 'a,>(&mut self, optional: Option<T>, kind: QueryWhereType) {
        if let Some(value) = optional {
            let column = match kind {
                QueryWhereType::Equal(name) => format!("{} = ?", name),
                QueryWhereType::Like(name) => format!("{} like ?", name),
                QueryWhereType::Custom(custom) => custom,
            };
            self.columns_update.push(column);
            self.values_update.push(Box::new(value));
        } 
    }

    pub fn add_where<T: ToSql + 'a,>(&mut self, optional: Option<T>, kind: QueryWhereType) {
        if let Some(value) = optional {
            let column = match kind {
                QueryWhereType::Equal(name) => format!("{} = ?", name),
                QueryWhereType::Like(name) => format!("{} like ?", name),
                QueryWhereType::Custom(custom) => custom,
            };
            self.columns_where.push(column);
            self.values_where.push(Box::new(value));
        } 
    }


    pub fn format_update(&self) -> String {
        if self.columns_update.len() > 0 {
            self.columns_update.join(", ")
        } else {
            "".to_string()
        }
    }
    
    pub fn format(&self) -> String {
        if self.columns_where.len() > 0 {
            format!("WHERE {}", self.columns_where.join(" and "))
        } else {
            "".to_string()
        }
    }

    pub fn values(&mut self) -> ParamsFromIter<&Vec<Box<dyn ToSql + 'a>>> {
        let all_values = &mut self.values_update;
        all_values.append(&mut self.values_where);
        /*for value in &mut *all_values {
            println!("{:?}", value.to_sql())
        }*/
        params_from_iter(all_values)
    }
}

