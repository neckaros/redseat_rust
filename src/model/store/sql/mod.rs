pub mod libraries;
pub mod users;
pub mod credentials;
pub mod backups;
pub mod library;
pub mod plugins;

use std::fmt::Display;

use rsa::{pkcs8::der::TagNumber, rand_core::le};
use rusqlite::{params_from_iter, types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef}, ParamsFromIter, Row, ToSql};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use strum::additional_attributes;
use strum_macros::EnumString;
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


pub enum SqlWhereType {
    Like(String, Box<dyn ToSql>),
    Equal(String, Box<dyn ToSql>),
    After(String, Box<dyn ToSql>),
    Before(String, Box<dyn ToSql>),
    Between(String, Box<dyn ToSql>, Box<dyn ToSql>),
    Custom(String, Box<dyn ToSql>),
    In(String, Vec<Box<dyn ToSql>>),
    NotIn(String, Vec<Box<dyn ToSql>>),
    Static(String),
    SeparatedContain(String, String, Box<dyn ToSql>),
    InStringList(String, String, Box<dyn ToSql>),
    EqualWithAlt(String, String, String, Box<dyn ToSql>),
    Or(Vec<SqlWhereType>),
    And(Vec<SqlWhereType>),
}


impl SqlWhereType {
    pub fn expand(&self) -> Result<(String, Vec<&Box<dyn ToSql>>)> {
        let mut values: Vec<&Box<dyn ToSql>> = vec![];
        let text = match self {
            SqlWhereType::Equal(name, value) => {
                values.push(value);
                format!("{} = ?", name)
            },
            SqlWhereType::Like(name, value) => {
                values.push(value);
                format!("{} like ?", name)
            },
            SqlWhereType::Custom(custom, value) => {
                values.push(value);
                custom.to_string()
            },
            SqlWhereType::After(name, value) => {
                values.push(value);
                format!("{} > ?", name)
            },
            SqlWhereType::Before(name, value) => {
                values.push(value);
                format!("{} < ?", name)
            },
            
            SqlWhereType::Between(name, down, up) => {
                values.push(down);
                values.push(up);
                format!("{} BETWEEN ? and ?", name)
            },
            SqlWhereType::In(name, ins) => {

                for value in ins {
                    values.push(value);
                }
                format!("{} in ({})", name, ins.iter().map(|_| "?").collect::<Vec<_>>().join(", "))
            },
            SqlWhereType::NotIn(name, ins) => {

                for value in ins {
                    values.push(value);
                }
                format!("{} not in ({})", name, ins.iter().map(|_| "?").collect::<Vec<_>>().join(", "))
            },
            
            SqlWhereType::InStringList(name, separator, value) => {

                values.push(value);

                format!("('{}' || {} || '{}' LIKE '%{}' || ? || '{}%')", separator, name, separator, separator, separator)
            },
            SqlWhereType::EqualWithAlt(name, alt, separator, value) => {

                values.push(value);
                values.push(value);

                format!("( {} = ? COLLATE NOCASE or  '{}' || {} || '{}' LIKE '%{}' || ? || '{}%')", name, separator, alt, separator, separator, separator)
            },
            SqlWhereType::SeparatedContain(name, separator, value) => {
                values.push(value);

                format!("'{}' || {} || '{}' LIKE '%{}' || ? || '{}%'", separator, name, separator, separator, separator)
            },
            SqlWhereType::Static(s) => {
                s.to_string()
            },
            SqlWhereType::Or(sub_queries) => {
                let mut texts: Vec<String> = vec![];
                for query in sub_queries {
                    let (t, mut v) = query.expand()?;
                    texts.push(t);
                    values.append(&mut v);
                }
                format!("({})", texts.join(" or "))
            },
            SqlWhereType::And(sub_queries) => {
                let mut texts: Vec<String> = vec![];
                for query in sub_queries {
                    let (t, mut v) = query.expand()?;
                    texts.push(t);
                    values.append(&mut v);
                }
                format!("({})", texts.join(" and "))
            },
        };
        Ok((text, values))
    }
}







pub enum QueryWhereType<'a> {
    Like(&'a str, &'a dyn ToSql),
    Equal(&'a str, &'a dyn ToSql),
    After(&'a str, &'a dyn ToSql),
    Before(&'a str, &'a dyn ToSql),
    Custom(&'a str, &'a dyn ToSql),
    In(&'a str, Vec<&'a dyn ToSql>),
    NotIn(&'a str, Vec<&'a dyn ToSql>),
    Static(String),
    SeparatedContain(&'a str, String, &'a dyn ToSql),
    InStringList(&'a str, &'a str, &'a dyn ToSql),
    EqualWithAlt(&'a str, &'a str, &'a str, &'a dyn ToSql),
    Or(Vec<QueryWhereType<'a>>),
    And(Vec<QueryWhereType<'a>>),
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
            QueryWhereType::In(name, ins) => {

                for value in ins {
                    values.push(value);
                }
                format!("{} in ({})", name, ins.iter().map(|_| "?").collect::<Vec<_>>().join(", "))
            },
            QueryWhereType::NotIn(name, ins) => {

                for value in ins {
                    values.push(value);
                }
                format!("{} not in ({})", name, ins.iter().map(|_| "?").collect::<Vec<_>>().join(", "))
            },
            
            QueryWhereType::InStringList(name, separator, value) => {

                values.push(value);

                format!("('{}' || {} || '{}' LIKE '%{}' || ? || '{}%')", separator, name, separator, separator, separator)
            },
            QueryWhereType::EqualWithAlt(name, alt, separator, value) => {

                values.push(value);
                values.push(value);

                format!("( {} = ? COLLATE NOCASE or  '{}' || {} || '{}' LIKE '%{}' || ? || '{}%')", name, separator, alt, separator, separator, separator)
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
            QueryWhereType::And(sub_queries) => {
                let mut texts: Vec<String> = vec![];
                for query in sub_queries {
                    let (t, mut v) = query.expand()?;
                    texts.push(t);
                    values.append(&mut v);
                }
                format!("({})", texts.join(" and "))
            },
        };
        Ok((text, values))
    }
}



#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, strum_macros::Display,EnumString, Default)]
#[strum(serialize_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum SqlOrder {
    ASC,
    #[default]
    DESC
}

#[derive(Debug)]
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

#[derive(Default)]
pub struct RsQueryBuilder {

    wheres: Vec<SqlWhereType>,

    columns_recursive: Vec<String>,
    values_recursive: Vec<Box<dyn ToSql>>,

    columns_update: Vec<String>,
    values_update: Vec<Box<dyn ToSql>>,

    columns_orders: Vec<OrderBuilder>,
}

impl RsQueryBuilder {
    pub fn new() -> Self {
        RsQueryBuilder::default()
    }
    pub fn add_where(&mut self, kind: SqlWhereType) {
        self.wheres.push(kind);
    }    
    pub fn add_update<T: ToSql>(&mut self, optional: Option<Box<dyn ToSql>>, column: &str)  {
        if let Some(value) = optional {
            self.columns_update.push(format!("{} = ?", column));
            self.values_update.push(value);
        }
    }

    pub fn add_nullify(&mut self, column: &str)  {
        self.columns_update.push(format!("{} = NULL", column));
    }


    pub fn add_recursive<T: ToSql + Display + 'static>(&mut self, table: String, mapping_table: String, map_key: String, map_field: String, id: Box<T>, additional_filter: Option<String>) {
        let table_name = format!("{}_{}", table, id.to_string().replace("-", "_"));

        let sql = format!("{}(n) AS (
            VALUES(?)
            UNION
            SELECT id FROM {}, {}
             WHERE {}.parent={}.n)", table_name, table, table_name, table, table_name);
        self.columns_recursive.push(sql);
        self.values_recursive.push(id);
        self.wheres.push(SqlWhereType::Static(format!("id IN (SELECT tm.{} FROM {} tm WHERE {} IN {}{})", map_key, mapping_table, map_field, table_name, additional_filter.unwrap_or("".to_string()))));
    }    
    pub fn format_recursive(&self) -> String {
        if !self.columns_recursive.is_empty() {
            format!("WITH RECURSIVE {} ", self.columns_recursive.join(", "))
        } else {
            "".to_string()
        }
    }


    pub fn format(&self) -> String {
        if !self.wheres.is_empty() {
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
    pub fn values(&mut self) -> ParamsFromIter<Vec<&Box<dyn ToSql>>> {
        let mut all_values = vec![];
        all_values.append(&mut self.values_recursive.iter().collect());
        //all_values.append(&mut self.values_update);

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

    pub fn add_nullify(&mut self, column: &str)  {
        self.columns_update.push(format!("{} = NULL", column));
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