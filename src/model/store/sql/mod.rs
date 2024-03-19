pub mod libraries;
pub mod users;
pub mod credentials;
pub mod backups;
pub mod library;
pub mod plugins;

use std::fmt::Display;

use rsa::{pkcs8::der::TagNumber, rand_core::le};
use rusqlite::{params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, ParamsFromIter, Row, ToSql};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio_rusqlite::Connection;


use super::Result;


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
            if version < 3 {
                let update = String::from_utf8_lossy(include_bytes!("003 - AI MODELS.sql"));
                conn.execute_batch(&update)?;
                
                conn.pragma_update(None, "user_version", 3)?;
                println!("Update SQL to verison 3")
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



pub enum QueryWhereType<'a> {
    Like(&'a str, &'a dyn ToSql),
    Equal(&'a str, &'a dyn ToSql),
    After(&'a str, &'a dyn ToSql),
    Before(&'a str, &'a dyn ToSql),
    Custom(&'a str, &'a dyn ToSql),
    Static(String),
    SeparatedContain(&'a str, String, &'a dyn ToSql),
    EqualWithAlt(&'a str, &'a str, &'a str, &'a dyn ToSql),
    Or(Vec<QueryWhereType<'a>>),
}

impl<'a> QueryWhereType<'a> {
    pub fn expand(&'a self) -> Result<(String, Vec<&'a dyn ToSql>)> {
        let mut values: Vec<&'a dyn ToSql> = vec![];
        let text = match self {
            QueryWhereType::Equal(name, value) => {
                values.push(value);
                format!("{} = ?", name)
            },
            QueryWhereType::Like(name, value) => {
                values.push(value);
                format!("{} like ?", name)
            },
            QueryWhereType::Custom(custom, value) => {
                values.push(value);
                custom.to_string()
            },
            QueryWhereType::After(name, value) => {
                values.push(value);
                format!("{} > ?", name)
            },
            QueryWhereType::Before(name, value) => {
                values.push(value);
                format!("{} < ?", name)
            },
            QueryWhereType::EqualWithAlt(name, alt, separator, value) => {

                values.push(value);
                values.push(value);

                format!("( {} = ? or  '{}' || {} || '{}' LIKE '%{}' || ? || '{}%')", name, separator, alt, separator, separator, separator)
            },
            QueryWhereType::SeparatedContain(name, separator, value) => {
                values.push(value);

                format!("'{}' || {} || '{}' LIKE '%{}' || ? || '{}%'", separator, name, separator, separator, separator)
            },
            QueryWhereType::Static(s) => {
                s.to_string()
            },
            QueryWhereType::Or(sub_queries) => {
                let mut texts: Vec<String> = vec![];
                for query in sub_queries {
                    let (t, mut v) = query.expand()?;
                    texts.push(t);
                    values.append(&mut v);
                }
                format!("({})", texts.join(" or "))
            },
        };
        Ok((text, values))
    }
}

pub enum SqlOrder {
    ASC,
    DESC
}

pub struct OrderBuilder {
    column: String,
    order: SqlOrder
}

impl OrderBuilder {
    pub fn new(column: String, order: SqlOrder) -> Self {
        OrderBuilder { column, order }
    }
    pub fn format(&self) -> String {
        match self.order {
            SqlOrder::ASC => self.column.clone(),
            SqlOrder::DESC => format!("{} DESC", self.column),
        }
    }
}

pub struct QueryBuilder<'a> {

    wheres: Vec<QueryWhereType<'a>>,

    columns_recursive: Vec<String>,
    values_recursive: Vec<&'a (dyn ToSql + 'a)>,
    columns_update: Vec<String>,
    values_update: Vec<&'a dyn ToSql>,
    
    columns_orders: Vec<OrderBuilder>,
}

impl <'a> QueryBuilder<'a> {
    pub fn new() -> Self {
        Self {
            wheres: Vec::new(),
            columns_recursive: Vec::new(),
            values_recursive: Vec::new(),
            columns_update: Vec::new(),
            values_update: Vec::new(),
            columns_orders: Vec::new()
        }
    }

    pub fn add_recursive<T: ToSql + Display>(&mut self, table: &str, mapping_table: &str, map_key: &str, map_field: &str, id: &'a T) {
            let table_name = format!("{}_{}", table, id.to_string().replace("-", "_"));

            let sql = format!("{}(n) AS (
                VALUES(?)
                UNION
                SELECT id FROM {}, {}
                 WHERE {}.parent={}.n)", table_name, table, table_name, table, table_name);
            self.columns_recursive.push(sql);
            self.values_recursive.push(id);
            self.wheres.push(QueryWhereType::Static(format!("id IN (SELECT tm.{} FROM {} tm WHERE {} IN {})", map_key, mapping_table, map_field, table_name)));
            //self.columns_where.push(format!("id IN (SELECT tm.{} FROM {} tm WHERE {} IN {})", map_key, mapping_table, map_field, table_name))
           
        
    }

    pub fn add_update<T: ToSql>(&mut self, optional: &'a Option<T>, column: &str)  {
        if let Some(value) = optional {
            self.columns_update.push(format!("{} = ?", column));
            self.values_update.push(value);
        }
    }

    pub fn add_where(&mut self, kind: QueryWhereType<'a>) {
        self.wheres.push(kind);
    }


    pub fn format_update(&self) -> String {
        if self.columns_update.len() > 0 {
            self.columns_update.join(", ")
        } else {
            "".to_string()
        }
    }
    
    pub fn format(&self) -> String {
        if self.wheres.len() > 0 {
            let mut columns = vec![];
            for w in &self.wheres {
                let (t, _) = w.expand().unwrap();
                columns.push(t);
            }
            format!("WHERE {}", columns.join(" and "))
        } else {
            "".to_string()
        }
    }

    pub fn add_oder(&mut self, order: OrderBuilder) {
        self.columns_orders.push(order);
    }

    pub fn format_order(&self) -> String {
        if self.columns_orders.len() > 0 {
            format!(" ORDER BY {}", self.columns_orders.iter().map(|o| o.format()).collect::<Vec<String>>().join(", "))
        } else {
            "".to_string()
        }
    }

    pub fn format_recursive(&self) -> String {
        if self.columns_recursive.len() > 0 {
            format!("WITH RECURSIVE {} ", self.columns_recursive.join(", "))
        } else {
            "".to_string()
        }
    }

    pub fn values(&'a mut self) -> ParamsFromIter<&Vec<&'a (dyn ToSql + 'a)>> {
        let all_values = &mut self.values_recursive;
        all_values.append(&mut self.values_update);

        for w in &self.wheres {
            let r = w.expand();
            let (_, mut v) = r.unwrap();
            all_values.append(&mut v);
        }
        /*for value in &mut *all_values {
            println!("{:?}", value.to_sql())
        }*/
        params_from_iter(all_values)
    }
}


pub fn deserialize_from_row<T: DeserializeOwned>(row: &Row, index: usize) -> Result<T> {
    let value: Value = row.get(index)?;

    let u = serde_json::from_value::<T>(value)?;
    Ok(u)
}